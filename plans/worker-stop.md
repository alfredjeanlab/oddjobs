# Plan: `oj worker stop <name>`

## Overview

Add an `oj worker stop <name>` CLI subcommand that sends a `WorkerStop` request to the daemon. The daemon marks the worker as stopped so it no longer dispatches new queue items. Existing active pipelines continue running to completion — they are **not** cancelled.

Most of the infrastructure already exists: `Request::WorkerStop`, `Event::WorkerStopped`, `handle_worker_stop()` in the listener, and `handle_worker_stopped()` in the runtime. The main work is:

1. Adding the CLI subcommand
2. Changing the runtime handler to **not** cancel active pipelines (current behavior cancels them)
3. Updating tests to match the new semantics

## Project Structure

```
crates/
├── cli/src/commands/worker.rs       # ADD Stop variant to WorkerCommand enum
├── engine/src/runtime/handlers/
│   └── worker.rs                    # MODIFY handle_worker_stopped() — stop cancelling pipelines
├── engine/src/runtime_tests/
│   └── worker.rs                    # MODIFY existing test, add new test
└── storage/src/state.rs             # (no change — already sets status="stopped")
```

## Dependencies

None. All required protocol types, events, and daemon handler already exist.

## Implementation Phases

### Phase 1: Add CLI subcommand

**File:** `crates/cli/src/commands/worker.rs`

Add a `Stop` variant to `WorkerCommand` and handle it in the `handle()` function.

```rust
#[derive(Subcommand)]
pub enum WorkerCommand {
    /// Start a worker (idempotent: wakes it if already running)
    Start { name: String },
    /// Stop a worker (active pipelines continue, no new items dispatched)
    Stop { name: String },
    /// List all workers and their status
    List {},
}
```

In the `handle()` match arm:

```rust
WorkerCommand::Stop { name } => {
    let request = Request::WorkerStop {
        worker_name: name.clone(),
        namespace: namespace.to_string(),
    };
    match client.send(&request).await? {
        Response::Ok => {
            println!("Worker '{}' stopped", name);
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => {
            anyhow::bail!("unexpected response from daemon");
        }
    }
}
```

**Verify:** `cargo build --all` compiles. `oj worker stop --help` shows the subcommand.

### Phase 2: Change runtime to not cancel active pipelines on stop

**File:** `crates/engine/src/runtime/handlers/worker.rs`

The current `handle_worker_stopped()` (line 437) drains `active_pipelines` and cancels each one. Change it to only set the status to `Stopped`, leaving active pipelines untouched. The worker won't pick up new items because all poll/dispatch paths already check `status != WorkerStatus::Stopped`.

```rust
pub(crate) async fn handle_worker_stopped(
    &self,
    worker_name: &str,
) -> Result<Vec<Event>, RuntimeError> {
    let mut workers = self.worker_states.lock();
    if let Some(state) = workers.get_mut(worker_name) {
        state.status = WorkerStatus::Stopped;
    }
    Ok(vec![])
}
```

Active pipelines remain in `state.active_pipelines`. When each completes, `check_worker_pipeline_complete()` will remove it from the set. Since the worker is stopped, the `should_poll` check at line 596-605 will return `false`, so no re-poll occurs — exactly the desired behavior.

**Verify:** `cargo test --all` passes (after Phase 3 test updates).

### Phase 3: Update tests

**File:** `crates/engine/src/runtime_tests/worker.rs`

#### 3a: Update existing `worker_stop_cancels_all_active_pipelines` test

Rename to `worker_stop_leaves_active_pipelines_running` and change assertions: after `WorkerStopped`, active pipelines should **not** be cancelled. The worker status should be `Stopped` and `active_pipelines` should still contain the dispatched pipeline IDs.

```rust
/// Stopping a worker marks it stopped but lets active pipelines finish.
#[tokio::test]
async fn worker_stop_leaves_active_pipelines_running() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 2);

    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 2).await;
    assert_eq!(count_dispatched(&events), 2);

    let dispatched = dispatched_pipeline_ids(&events);

    let stop_events = ctx
        .runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // No pipelines should be cancelled
    let cancelled_count = stop_events
        .iter()
        .filter(|e| matches!(e, Event::PipelineAdvanced { step, .. } if step == "cancelled"))
        .count();
    assert_eq!(cancelled_count, 0, "stop should not cancel active pipelines");

    // Worker should be stopped but still tracking active pipelines
    let workers = ctx.runtime.worker_states.lock();
    let state = workers.get("fixer").unwrap();
    assert_eq!(state.status, WorkerStatus::Stopped);
    assert_eq!(state.active_pipelines.len(), 2);
    for pid in &dispatched {
        assert!(state.active_pipelines.contains(pid));
    }
}
```

#### 3b: Add test for no new dispatch after stop

Verify that after stopping, a `WorkerWake` or `WorkerPollComplete` with new items does not dispatch new pipelines.

```rust
/// A stopped worker should not dispatch new items on wake.
#[tokio::test]
async fn stopped_worker_does_not_dispatch_on_wake() {
    let ctx = setup_with_runbook(CONCURRENT_WORKER_RUNBOOK).await;
    push_persisted_items(&ctx, "bugs", 1);

    // Start and dispatch first item
    let events = start_worker_and_poll(&ctx, CONCURRENT_WORKER_RUNBOOK, "fixer", 1).await;
    assert_eq!(count_dispatched(&events), 1);

    // Stop the worker
    ctx.runtime
        .handle_event(Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // Push more items and try to wake
    push_persisted_items(&ctx, "bugs", 1);
    let wake_events = ctx
        .runtime
        .handle_event(Event::WorkerWake {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        })
        .await
        .unwrap();

    // No new dispatches should happen
    assert_eq!(count_dispatched(&wake_events), 0);
}
```

**Verify:** `cargo test --all` passes.

## Key Implementation Details

- **No new types or events needed.** The `WorkerStop` request, `WorkerStopped` event, and `WorkerStatus::Stopped` enum all exist already.
- **Dispatch guards already in place.** Both `handle_worker_wake()` (line 147) and `handle_worker_poll_complete()` (line 263) already check `status != WorkerStatus::Stopped` and bail early. Similarly, `check_worker_pipeline_complete()` checks `status == WorkerStatus::Running` before re-polling. So changing `handle_worker_stopped()` to not cancel pipelines is the only runtime change needed.
- **Storage layer unchanged.** `state.rs` already sets `record.status = "stopped"` on `WorkerStopped` — no change needed.
- **Daemon listener unchanged.** `handle_worker_stop()` already emits `WorkerStopped` and returns `Response::Ok` — no change needed.
- **Idempotent restart.** After `oj worker stop`, running `oj worker start` again will re-emit `WorkerStarted`, which overwrites the in-memory state with `status: Running` and triggers a fresh poll — correctly resuming the worker.

## Verification Plan

1. `cargo build --all` — confirms compilation
2. `cargo clippy --all-targets --all-features -- -D warnings` — no new warnings
3. `cargo test --all` — all tests pass including updated/new worker tests
4. `cargo fmt --all -- --check` — formatting clean
5. Manual smoke test:
   - Start a worker: `oj worker start <name>`
   - Confirm it's running: `oj worker list` shows status=running
   - Stop it: `oj worker stop <name>`
   - Confirm stopped: `oj worker list` shows status=stopped
   - Active pipelines (if any) continue and complete normally
   - Push new items — they remain pending, not dispatched
   - Restart: `oj worker start <name>` resumes dispatching
