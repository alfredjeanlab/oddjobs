# Checkpoint Lock Contention — Future

## Problem

The checkpoint task (`daemon/src/main.rs:329-376`) acquires both
`state.lock()` and `event_wal.lock()` simultaneously to clone the
materialized state and read the WAL's processed sequence number:

```rust
let (state_clone, processed_seq) = {
    let state_guard = state.lock();
    let wal_guard = event_wal.lock();
    (state_guard.clone(), wal_guard.processed_seq())
};
```

While both locks are held, the event loop cannot `apply_event()` (needs
state lock) and the flush task cannot `flush()` (needs WAL lock). The
duration depends on the cost of `state_guard.clone()`, which grows with
the number of pipelines, step histories, and workspace records.

After the clone, the task saves the snapshot to disk (`serde_json::to_writer`
+ `sync_all`) and then re-acquires the WAL lock to truncate:

```rust
let mut wal = event_wal.lock();
wal.truncate_before(processed_seq)?;
```

The truncation rewrites the WAL file (temp + rename + sync), holding the
WAL lock for the full I/O duration.

## Current Impact

At current scale (tens of pipelines, small step histories), the state clone
completes in microseconds and the snapshot write in low milliseconds. No
stalls have been observed. The checkpoint runs every 60 seconds, so even
a brief stall occurs infrequently.

## When This Matters

This becomes a concern when:

- Hundreds of concurrent pipelines with large step histories make the
  state clone expensive (10ms+)
- Large WAL files make truncation slow on rotational storage
- The 60-second interval coincides with a long-running effect chain,
  compounding the stall

## Potential Fix

Decouple the locks:

1. Read `processed_seq` from the WAL lock (release immediately)
2. Clone state from the state lock (release immediately)
3. Save snapshot without any lock held
4. Acquire WAL lock only for truncation

```rust
let processed_seq = {
    let wal = event_wal.lock();
    wal.processed_seq()
};
let state_clone = {
    let state = state.lock();
    state.clone()
};

// No locks held during I/O
snapshot.save(&state_clone, processed_seq)?;

let mut wal = event_wal.lock();
wal.truncate_before(processed_seq)?;
```

The snapshot may be slightly inconsistent (state cloned after WAL seq read,
so it could include one extra event). This is harmless — on recovery, the
WAL replay is idempotent and `apply_event` handles duplicates.

For the truncation I/O, consider doing the file rewrite to a temp path
without the lock, then acquiring the lock only for the atomic rename and
in-memory state update.
