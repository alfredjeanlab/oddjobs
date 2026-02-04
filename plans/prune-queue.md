# Plan: Queue Prune Command

## Overview

Add `oj queue prune <queue>` to remove completed (and dead) items from a persisted queue. By default, only items older than 12 hours are pruned. The `--all` flag removes all terminal items regardless of age. This follows the same conventions as the existing `oj pipeline prune`, `oj worker prune`, and `oj cron prune` commands.

Terminal queue item statuses eligible for pruning: **Completed** and **Dead**.

Items with status **Pending**, **Active**, or **Failed** are never pruned (failed items may still be retried).

## Project Structure

Files to create or modify:

```
crates/
├── cli/src/commands/queue.rs        # Add Prune variant + handler
├── cli/src/client_queries.rs        # Add queue_prune() helper
├── daemon/src/protocol.rs           # Add QueuePrune request + QueuesPruned response
├── daemon/src/protocol_status.rs    # Add QueueItemEntry struct
├── daemon/src/listener/mod.rs       # Route QueuePrune to handler
├── daemon/src/listener/queues.rs    # Implement handle_queue_prune()
└── daemon/src/listener/queues_tests.rs  # Unit tests
```

## Dependencies

No new external dependencies. All required functionality exists in the crate workspace.

## Implementation Phases

### Phase 1: Protocol Layer

Add the request/response types to `crates/daemon/src/protocol.rs` and `crates/daemon/src/protocol_status.rs`.

**protocol.rs — Request enum** (add near existing queue requests, after `QueueDrain`):

```rust
/// Prune completed/dead items from a persisted queue
QueuePrune {
    project_root: PathBuf,
    #[serde(default)]
    namespace: String,
    queue_name: String,
    /// Prune all terminal items regardless of age
    all: bool,
    /// Preview only — don't actually delete
    dry_run: bool,
},
```

**protocol.rs — Response enum** (add near other prune responses):

```rust
/// Queue prune result
QueuesPruned {
    pruned: Vec<QueueItemEntry>,
    skipped: usize,
},
```

**protocol_status.rs — new entry struct** (add after `CronEntry`):

```rust
/// Queue item entry for prune responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueItemEntry {
    pub queue_name: String,
    pub item_id: String,
    pub status: String,
}
```

Update the re-exports in `protocol.rs` to include `QueueItemEntry`.

### Phase 2: Daemon Handler

Implement `handle_queue_prune()` in `crates/daemon/src/listener/queues.rs`.

The handler:
1. Loads the runbook and validates the queue exists and is persisted (same pattern as `handle_queue_drop`).
2. Reads `state.queue_items` for the scoped queue name.
3. Filters for terminal items (status == `Completed` or `Dead`).
4. Unless `--all`, applies the 12-hour age threshold using `pushed_at_epoch_ms` (consistent with pipeline prune).
5. Unless `--dry-run`, emits a `QueueDropped` event for each item to remove (reuses the existing event — no new event type needed).
6. Returns `Response::QueuesPruned { pruned, skipped }`.

```rust
pub(super) fn handle_queue_prune(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    all: bool,
    dry_run: bool,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // 1. Load runbook, validate queue exists and is persisted
    //    (same pattern as handle_queue_drop / handle_queue_drain)

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let age_threshold_ms: u64 = 12 * 60 * 60 * 1000; // 12 hours

    // 2. Collect terminal items (Completed, Dead)
    let mut to_prune = Vec::new();
    let mut skipped = 0usize;
    {
        let state = state.lock();
        let key = scoped_name(namespace, queue_name);
        if let Some(items) = state.queue_items.get(&key) {
            for item in items {
                let is_terminal = matches!(
                    item.status,
                    QueueItemStatus::Completed | QueueItemStatus::Dead
                );
                if !is_terminal {
                    skipped += 1;
                    continue;
                }
                if !all && now_ms.saturating_sub(item.pushed_at_epoch_ms) < age_threshold_ms {
                    skipped += 1;
                    continue;
                }
                to_prune.push(QueueItemEntry {
                    queue_name: queue_name.to_string(),
                    item_id: item.id.clone(),
                    status: format!("{:?}", item.status).to_lowercase(),
                });
            }
        }
    }

    // 3. Emit QueueDropped events (unless dry-run)
    if !dry_run {
        for entry in &to_prune {
            let event = Event::QueueDropped {
                queue_name: queue_name.to_string(),
                item_id: entry.item_id.clone(),
                namespace: namespace.to_string(),
            };
            event_bus.send(event).map_err(|_| ConnectionError::WalError)?;
        }
    }

    Ok(Response::QueuesPruned {
        pruned: to_prune,
        skipped,
    })
}
```

### Phase 3: Listener Dispatch

Wire the new request in `crates/daemon/src/listener/mod.rs`.

Add a match arm for `Request::QueuePrune` in the main dispatch (near the existing `Request::QueueDrain` arm):

```rust
Request::QueuePrune {
    project_root,
    namespace,
    queue_name,
    all,
    dry_run,
} => queues::handle_queue_prune(
    &project_root,
    &namespace,
    &queue_name,
    all,
    dry_run,
    event_bus,
    state,
),
```

### Phase 4: CLI Client Helper

Add `queue_prune()` to `crates/cli/src/client_queries.rs` (next to existing `worker_prune`, etc.):

```rust
/// Prune completed/dead items from a queue
pub async fn queue_prune(
    &self,
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    all: bool,
    dry_run: bool,
) -> Result<(Vec<oj_daemon::QueueItemEntry>, usize), ClientError> {
    let req = Request::QueuePrune {
        project_root: project_root.to_path_buf(),
        namespace: namespace.to_string(),
        queue_name: queue_name.to_string(),
        all,
        dry_run,
    };
    match self.send(&req).await? {
        Response::QueuesPruned { pruned, skipped } => Ok((pruned, skipped)),
        other => Self::reject(other),
    }
}
```

### Phase 5: CLI Command

Add the `Prune` variant to `QueueCommand` in `crates/cli/src/commands/queue.rs`:

```rust
/// Remove completed and dead items from a queue
Prune {
    /// Queue name
    queue: String,
    /// Prune all terminal items regardless of age
    #[arg(long)]
    all: bool,
    /// Show what would be pruned without making changes
    #[arg(long)]
    dry_run: bool,
},
```

Add the handler arm in the `handle()` match block:

```rust
QueueCommand::Prune { queue, all, dry_run } => {
    let (pruned, skipped) = client
        .queue_prune(project_root, namespace, &queue, all, dry_run)
        .await?;

    print_prune_results(
        &pruned,
        skipped,
        dry_run,
        format,
        "queue item",
        "skipped",
        |entry| {
            format!(
                "item {} ({}) from queue '{}'",
                &entry.item_id[..8.min(entry.item_id.len())],
                entry.status,
                entry.queue_name,
            )
        },
    )?;
}
```

Add `print_prune_results` to the existing import from `crate::output`.

### Phase 6: Tests

Add unit tests in `crates/daemon/src/listener/queues_tests.rs` covering:

1. **Prune completed items older than 12h** — push items, mark completed with old timestamps, verify they are pruned.
2. **Skip recent completed items** — completed items within 12h are skipped (not pruned).
3. **--all flag prunes recent items** — completed items within 12h are pruned when `all: true`.
4. **Prune dead items** — dead items are pruned alongside completed.
5. **Skip active/pending/failed items** — these statuses are never pruned, counted as skipped.
6. **Dry-run mode** — items are listed in the response but no `QueueDropped` events are emitted (state unchanged).
7. **Empty queue / no terminal items** — returns `pruned: [], skipped: 0` (or appropriate skipped count).

Follow the existing test pattern: create a `TestProject`, push items via `handle_queue_push`, manipulate timestamps in state, then call `handle_queue_prune` and assert on the response and state.

## Key Implementation Details

- **Age threshold**: 12 hours (consistent with `pipeline prune`), based on `pushed_at_epoch_ms`. Queue items don't have a `completed_at_epoch_ms` field, so push time is the best available proxy. This is acceptable because completed items are always older than their push time.
- **Event reuse**: Uses existing `QueueDropped` event to remove items — no new event type or WAL schema change needed. The state handler for `QueueDropped` already removes items via `retain()`.
- **Runbook validation**: The handler validates the queue exists in the runbook and is persisted, consistent with `queue drop` and `queue drain`.
- **`scoped_name()`**: Uses `oj_core::namespace::scoped_name()` to build the `namespace::queue_name` key for `state.queue_items`.
- **Output**: Reuses the shared `print_prune_results` helper from `crates/cli/src/output.rs`.
- **No `--project` filter**: Unlike `pipeline prune`, the queue name is a required positional arg that already scopes to a single queue within the project namespace.

## Verification Plan

1. `cargo fmt --all` — formatting
2. `cargo clippy --all -- -D warnings` — lint
3. `cargo build --all` — compilation
4. `cargo test --all` — all tests pass, including new tests in `queues_tests.rs`
5. Manual smoke test:
   - `oj queue show <name>` — see completed/dead items
   - `oj queue prune <name> --dry-run` — preview what would be pruned
   - `oj queue prune <name>` — prune old terminal items
   - `oj queue prune <name> --all` — prune all terminal items regardless of age
   - `oj queue show <name>` — confirm items removed
