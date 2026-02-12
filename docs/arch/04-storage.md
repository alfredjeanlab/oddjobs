# Storage Layer

Write-ahead log (WAL) for durable state persistence with crash recovery.

## Architecture

```diagram
Event → WAL (append + fsync) → Materialized State
                  ↓
            Snapshots (periodic)
```

State is derived from WAL. On startup, load latest snapshot then replay WAL entries.

## WAL Entry Format

JSONL format — one JSON object per line:

```
{"seq":1,"event":{"type":"job:created","id":"p1","kind":"build",...}}\n
{"seq":2,"event":{"type":"step:completed","job_id":"p1","step":"build"}}\n
```

- **seq**: Monotonic sequence number, never repeats
- **event**: JSON-serialized `Event` from oj-core (tagged via `{"type": "event:name", ...fields}`)

The WAL stores core `Event` values directly. State mutations use typed `Event` variants (e.g., `JobCreated`, `StepFailed`) emitted via `Effect::Emit`.

### Group Commit

Writes are buffered in memory and flushed to disk either:
- On interval (~10ms)
- When the buffer reaches 100 entries
- Explicitly via `flush()`

A single `fsync` covers the entire batch for performance.

## State Mutation Events

State mutations use typed `Event` variants emitted via `Effect::Emit`. These events are written to WAL and applied by `MaterializedState::apply_event()`. See [Events](../interface/EVENTS.md) for the complete list of event types and their categories.

Events fall into three categories:

- **State mutations**: Applied by `apply_event()` to update materialized state (e.g., `job:created`, `agent:spawned`, `step:completed`, `worker:started`, `queue:pushed`, `decision:created`, `crew:created`)
- **Signals**: Handled by the runtime but do not mutate state (e.g., `command:run`, `timer:start`, `agent:idle`, `agent:prompt`)
- **Actions**: Emitted externally to trigger runtime operations (e.g., `job:resume`, `job:cancel`, `agent:signal`, `agent:input`)

All events (including signals and actions) are persisted to WAL. `CommandRun` persists the project → project_path mapping but is otherwise a signal.

## Materialized State

State is rebuilt by replaying events:

```rust
pub struct MaterializedState {
    pub jobs: HashMap<String, Job>,
    pub workspaces: HashMap<String, Workspace>,
    pub runbooks: HashMap<String, StoredRunbook>,
    pub workers: HashMap<String, WorkerRecord>,
    pub queue_items: HashMap<String, Vec<QueueItem>>,
    pub crons: HashMap<String, CronRecord>,
    pub decisions: HashMap<String, Decision>,
    pub crew: HashMap<String, Crew>,
    pub agents: HashMap<String, AgentRecord>,      // Unified agent index (agent_id → record)
    pub project_paths: HashMap<String, PathBuf>,   // Namespace → project path mapping
}
```

Each event type has logic in `apply_event()` that updates state deterministically.

Workers and queues use project-scoped composite keys (`project/name`) so that multiple projects can define identically-named resources without collision.

## Snapshots

Periodic snapshots compress history:

```rust
pub struct Snapshot {
    pub version: u32,                // Schema version for migrations
    pub seq: u64,                    // WAL sequence at snapshot time
    pub state: MaterializedState,
    pub created_at: DateTime<Utc>,
}
```

Recovery: Load snapshot, migrate if needed, replay only entries after `snapshot.seq`.

### Checkpoint Flow

Checkpoints run every 60 seconds with I/O off the main thread:

```diagram
Main Thread (async)           Background Thread
─────────────────────────     ─────────────────────────────
clone state (~10ms)
  │
  └─────────────────────────→ serialize JSON (~100ms)
                              compress with zstd (~30ms)
                              write to .tmp (~20ms)
                              fsync .tmp (~50ms)
                              rename → snapshot.json (~1ms)
                              fsync directory (~30ms)
                                │
  ←─────────────────────────────┘ (completion signal)
truncate WAL (safe now)
```

**Critical invariant**: WAL truncation only happens after directory fsync. This ensures the snapshot rename survives power loss — without it, a crash could leave the old snapshot on disk while the WAL has been truncated, losing events.

### Compression

Snapshots use zstd compression (level 3) for ~70-80% size reduction. Snapshots are always zstd-compressed; the loader expects compressed format.

### Testability

The `CheckpointWriter` trait abstracts all I/O, enabling tests to verify ordering (fsync before rename, dir fsync before WAL truncation), inject failures at any step, and test crash recovery without touching the filesystem.

### Versioning and Migrations

Snapshots include a schema version. On load, migrations are applied sequentially until the current version. Migrations transform JSON in place via `fn(&mut Value) -> Result<(), MigrationError>`, allowing schema evolution without maintaining legacy Rust types.

**Why migrations are required**: WAL is truncated after checkpoint, so "discard snapshot and replay WAL" would lose all state before the snapshot. Migrations must succeed or the daemon fails to start.

| Scenario | Behavior |
|----------|----------|
| Old snapshot, new daemon | Migrate forward, load normally |
| New snapshot, old daemon | Fail with `MigrationError::TooNew` |
| Migration failure | Daemon startup fails (no silent data loss) |

## Compaction

On each checkpoint (every 60 seconds):
1. Take snapshot at current processed sequence (overwrites previous snapshot)
2. Rewrite WAL keeping only entries >= snapshot sequence (write to `.tmp`, fsync, atomic rename)

## Corruption Handling

| Problem | Detection | Recovery |
|---------|-----------|----------|
| Corrupt WAL entry | JSON parse fails during scan | Rotate WAL to `.bak`, preserve valid entries before corruption in a new clean WAL |
| Corrupt WAL (read) | JSON parse fails in `next_unprocessed` | Log warning, skip corrupt line, advance read offset |
| Corrupt snapshot | JSON parse fails on load | Move snapshot to `.bak`, recover via full WAL replay |
| Invalid UTF-8 in WAL | `InvalidData` IO error | Stop reading at corruption point |

Backup rotation keeps up to 3 `.bak` files (`.bak`, `.bak.2`, `.bak.3`), removing the oldest when the limit is reached.

## Invariants

- Flush (with fsync) is the durability point -- buffered writes are not durable until flushed
- Sequence numbers are monotonically increasing and never repeat
- Replaying same entries produces identical state
