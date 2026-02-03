# Worker Prune

## Overview

Add an `oj worker prune` CLI command that removes stopped workers from daemon state. This follows the established pattern from `oj pipeline prune` and `oj agent prune`. The command sends a `WorkerPrune` request to the daemon, which removes all workers with `status == "stopped"` from materialized state by emitting `WorkerDeleted` events. The CLI reports how many workers were pruned. This also cleans up ghost workers from before namespace support that persist in snapshots with empty namespace and show as `(default)` in `oj status`.

## Project Structure

Files to create or modify:

```
crates/core/src/event.rs              # Add WorkerDeleted event variant
crates/storage/src/state.rs           # Handle WorkerDeleted in apply()
crates/daemon/src/protocol.rs         # Add Request::WorkerPrune, Response::WorkersPruned, WorkerEntry
crates/daemon/src/listener/mod.rs     # Route WorkerPrune to handler
crates/daemon/src/listener/mutations.rs  # Add handle_worker_prune()
crates/cli/src/client.rs              # Add worker_prune() client method
crates/cli/src/commands/worker.rs     # Add Prune subcommand and handler
crates/storage/src/state_tests.rs     # Unit test for WorkerDeleted event
docs/interface/EVENTS.md              # Document worker:deleted event
```

## Dependencies

No new external dependencies. All changes use existing crate infrastructure.

## Implementation Phases

### Phase 1: Core Event — `WorkerDeleted`

Add the `WorkerDeleted` event variant and its state application logic.

**`crates/core/src/event.rs`** — Add new variant after `WorkerStopped`:

```rust
#[serde(rename = "worker:deleted")]
WorkerDeleted {
    worker_name: String,
    #[serde(default)]
    namespace: String,
},
```

Also update the three match arms in the same file:
- `event_type()` → return `"worker:deleted"`
- `Display` impl → format as `"{t} worker={worker_name} ns={namespace}"`
- `pipeline_id()` → falls through to `None` (add to existing wildcard or explicit arm)

**`crates/storage/src/state.rs`** — In `apply()`, after the `WorkerStopped` arm:

```rust
Event::WorkerDeleted { worker_name, namespace } => {
    let key = scoped_key(namespace, worker_name);
    self.workers.remove(&key);
}
```

**`crates/engine/src/runtime/handlers/mod.rs`** — Add `Event::WorkerDeleted { .. }` to the no-op match arm (line ~239 area) alongside `PipelineDeleted` and other state-only events that the engine ignores.

**Verification:** `cargo check --all` passes. Add a unit test in `crates/storage/src/state_tests.rs` that applies `WorkerStarted` then `WorkerDeleted` and asserts the worker is removed from `state.workers`.

### Phase 2: Protocol Types

**`crates/daemon/src/protocol.rs`** — Add request, response, and entry types.

Request variant (alongside existing prune requests, ~line 119):

```rust
Request::WorkerPrune {
    all: bool,
    dry_run: bool,
},
```

Response variant (alongside existing prune responses, ~line 338):

```rust
Response::WorkersPruned {
    pruned: Vec<WorkerEntry>,
    skipped: usize,
},
```

Entry struct (alongside `PipelineEntry`, `AgentEntry`, etc.):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerEntry {
    pub name: String,
    pub namespace: String,
}
```

**Verification:** `cargo check --all` passes.

### Phase 3: Daemon Handler

**`crates/daemon/src/listener/mutations.rs`** — Add `handle_worker_prune()` following the `handle_pipeline_prune` pattern:

```rust
pub(super) fn handle_worker_prune(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    all: bool,
    dry_run: bool,
) -> Result<Response, ConnectionError> {
    let mut to_prune = Vec::new();
    let mut skipped = 0usize;

    {
        let state_guard = state.lock();
        for record in state_guard.workers.values() {
            if record.status != "stopped" {
                skipped += 1;
                continue;
            }
            to_prune.push(WorkerEntry {
                name: record.name.clone(),
                namespace: record.namespace.clone(),
            });
        }
    }

    if !dry_run {
        for entry in &to_prune {
            let event = Event::WorkerDeleted {
                worker_name: entry.name.clone(),
                namespace: entry.namespace.clone(),
            };
            event_bus.send(event).map_err(|_| ConnectionError::WalError)?;
        }
    }

    Ok(Response::WorkersPruned {
        pruned: to_prune,
        skipped,
    })
}
```

Note: No age threshold needed. Workers are either running or stopped — stopped workers are always eligible for pruning. The `--all` flag is accepted for CLI consistency but has no behavioral difference since all stopped workers are pruned by default. (If we later add a "recently stopped" grace period, `--all` will bypass it.)

**`crates/daemon/src/listener/mod.rs`** — Route the request (after the `WorkspacePrune` arm, ~line 259):

```rust
Request::WorkerPrune { all, dry_run } => {
    mutations::handle_worker_prune(state, event_bus, all, dry_run)
}
```

**Verification:** `cargo check --all` passes.

### Phase 4: CLI Client and Command

**`crates/cli/src/client.rs`** — Add client method after `workspace_prune()`:

```rust
pub async fn worker_prune(
    &self,
    all: bool,
    dry_run: bool,
) -> Result<(Vec<oj_daemon::WorkerEntry>, usize), ClientError> {
    let request = Request::WorkerPrune { all, dry_run };
    match self.send(&request).await? {
        Response::WorkersPruned { pruned, skipped } => Ok((pruned, skipped)),
        Response::Error { message } => Err(ClientError::Rejected(message)),
        _ => Err(ClientError::UnexpectedResponse),
    }
}
```

**`crates/cli/src/commands/worker.rs`** — Add `Prune` variant to `WorkerCommand`:

```rust
/// Remove stopped workers from daemon state
Prune {
    /// Prune all stopped workers (currently same as default)
    #[arg(long)]
    all: bool,

    /// Show what would be pruned without making changes
    #[arg(long)]
    dry_run: bool,
},
```

Add handler arm in `handle()`:

```rust
WorkerCommand::Prune { all, dry_run } => {
    let (pruned, skipped) = client.worker_prune(all, dry_run).await?;

    match format {
        OutputFormat::Text => {
            if dry_run {
                println!("Dry run — no changes made\n");
            }

            for entry in &pruned {
                let label = if dry_run { "Would prune" } else { "Pruned" };
                let ns = if entry.namespace.is_empty() {
                    "(default)".to_string()
                } else {
                    entry.namespace.clone()
                };
                println!("{} worker '{}' ({})", label, entry.name, ns);
            }

            let verb = if dry_run { "would be pruned" } else { "pruned" };
            println!(
                "\n{} worker(s) {}, {} skipped",
                pruned.len(),
                verb,
                skipped
            );
        }
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "dry_run": dry_run,
                "pruned": pruned,
                "skipped": skipped,
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
    }
}
```

**Verification:** `cargo check --all` and `cargo test --all` pass.

### Phase 5: Documentation and Final Verification

**`docs/interface/EVENTS.md`** — Add `worker:deleted` row to the Worker lifecycle table:

```
| `worker:deleted` | WorkerDeleted | `worker_name`, `namespace` |
```

Run full `make check` to verify everything passes (fmt, clippy, tests, build, audit, deny).

## Key Implementation Details

- **No age threshold**: Unlike pipeline/agent prune which have 12/24-hour grace periods, worker prune removes all stopped workers immediately. Workers are explicitly stopped and have no useful state to preserve once stopped.
- **Ghost worker cleanup**: Workers from before namespace support have `namespace: ""` and appear under `(default)` in `oj status`. These workers will have `status: "stopped"` (or possibly "running" if the daemon was killed without clean shutdown). The `--all` flag exists for forward-compatibility if a grace period is added later.
- **Event-driven state removal**: Following the `PipelineDeleted` pattern, worker removal goes through the event bus so it's captured in the WAL and survives daemon restarts. The `WorkerDeleted` event is the source of truth — `state.apply()` removes the worker from the HashMap.
- **Scoped keys**: Workers use `scoped_key(namespace, name)` as their HashMap key — `"namespace/worker_name"` or just `"worker_name"` for empty namespace. The `WorkerDeleted` event carries both `worker_name` and `namespace` so `apply()` can reconstruct the correct key.
- **No file cleanup needed**: Unlike pipeline/agent prune which delete log files, workers don't have their own log files to clean up.

## Verification Plan

1. **Unit test** (`crates/storage/src/state_tests.rs`): Apply `WorkerStarted` → `WorkerStopped` → `WorkerDeleted` events; assert worker is removed from state. Also test that deleting a worker with empty namespace works (ghost worker case).
2. **Compile check**: `cargo check --all` — ensures all new types, match arms, and imports are wired correctly.
3. **Full suite**: `make check` — fmt, clippy, tests, build, audit, deny all pass.
4. **Manual smoke test** (optional): Start daemon, start a worker, stop it, run `oj worker prune --dry-run` to see it listed, then `oj worker prune` to remove it, verify `oj worker list` no longer shows it.
