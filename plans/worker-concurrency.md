# Worker Concurrency > 1

## Overview

Enable workers to process multiple queue items in parallel by honoring the `concurrency` field in worker definitions. Currently, the infrastructure for concurrency > 1 already exists in the code (`WorkerState.concurrency`, `active_pipelines` tracking, available-slot calculation), but it has only been tested with `concurrency = 1`. This plan verifies the existing dispatch-loop logic works for concurrency > 1, fills any gaps in the stop/cleanup path, and adds comprehensive tests.

## Project Structure

Key files (all changes are within existing files):

```
crates/
├── engine/
│   └── src/
│       ├── runtime/
│       │   └── handlers/
│       │       ├── mod.rs           # Event dispatch (WorkerStopped handler)
│       │       └── worker.rs        # Worker lifecycle, poll, dispatch
│       └── runtime_tests/
│           └── worker.rs            # Worker unit tests
├── daemon/
│   └── src/
│       └── listener/
│           └── workers.rs           # WorkerStop request handler
└── storage/
    └── src/
        └── state.rs                 # WorkerRecord (active_pipeline_ids)
```

## Dependencies

No new external dependencies. All required infrastructure exists:
- `WorkerState.concurrency: u32` — already stored
- `WorkerState.active_pipelines: HashSet<PipelineId>` — already tracked
- Available-slot calculation in `handle_worker_poll_complete` (line 266-271) — already computes `concurrency - active`
- `items.iter().take(available_slots)` dispatch loop (line 294) — already iterates up to available slots
- Re-poll on pipeline completion in `check_worker_pipeline_complete` (line 584-620) — already checks `active < concurrency`

## Implementation Phases

### Phase 1: Audit and validate existing dispatch logic

**Goal**: Confirm the existing poll → dispatch → re-poll loop correctly handles concurrency > 1 without code changes.

The key code paths already support multiple concurrent pipelines:

1. **`handle_worker_poll_complete`** (worker.rs:266-271): Computes `available = concurrency - active_pipelines.len()` and dispatches up to `available` items in the loop at line 294. This already works for any concurrency value.

2. **`check_worker_pipeline_complete`** (worker.rs:584-594): When a pipeline completes, checks `active_pipelines.len() < concurrency` and triggers re-poll if there's capacity. This correctly handles partial completion (e.g., 1 of 2 slots freeing up).

3. **`poll_persisted_queue`** (worker.rs:188-236): Returns all pending items; the caller limits via `take(available_slots)`.

**Verification**: Write a test with `concurrency = 2` and 3 queued items. Confirm 2 are dispatched immediately and the 3rd waits.

### Phase 2: Fix worker stop to cancel active pipelines

**Goal**: When a worker is stopped, cancel all its active pipelines so resources are cleaned up.

Currently, `handle_worker_stopped` (worker.rs:436-445) only sets `status = Stopped` — it does **not** cancel active pipelines. With concurrency > 1, a stopped worker could leave multiple orphaned pipelines running.

**Changes in `crates/engine/src/runtime/handlers/worker.rs`**:

```rust
pub(crate) async fn handle_worker_stopped(
    &self,
    worker_name: &str,
) -> Result<Vec<Event>, RuntimeError> {
    let pipeline_ids: Vec<PipelineId> = {
        let mut workers = self.worker_states.lock();
        if let Some(state) = workers.get_mut(worker_name) {
            state.status = WorkerStatus::Stopped;
            state.active_pipelines.drain().collect()
        } else {
            vec![]
        }
    };

    let mut result_events = Vec::new();
    for pipeline_id in pipeline_ids {
        result_events.extend(self.handle_pipeline_cancel(&pipeline_id).await?);
    }
    Ok(result_events)
}
```

This drains `active_pipelines` and cancels each one. `handle_pipeline_cancel` already handles advancing the pipeline to the "cancelled" terminal step and cleaning up sessions/agents.

**Verification**: Test that stopping a worker with 2 active pipelines produces `PipelineCancel` events for both.

### Phase 3: Handle edge case — poll returns fewer items than capacity

**Goal**: Ensure workers re-poll when new items arrive even if the initial poll didn't fill all slots.

This already works via the `QueuePushed → WorkerWake → poll` chain (handlers/mod.rs:161-199). When a new item is pushed to a persisted queue, all running workers watching that queue receive a `WorkerWake` event, which triggers `handle_worker_wake` → `poll_persisted_queue` → `handle_worker_poll_complete`. The available-slot calculation will see the remaining capacity and dispatch.

For external queues, the same `WorkerWake` path triggers a `PollQueue` effect that re-runs the list command.

**No code changes needed** — just verify in tests.

### Phase 4: Add unit tests

**Goal**: Comprehensive test coverage for concurrency > 1 behavior.

Add the following tests to `crates/engine/src/runtime_tests/worker.rs`:

#### Test 1: `concurrency_2_dispatches_two_items_simultaneously`

```rust
// Runbook with concurrency = 2
const CONCURRENT_WORKER_RUNBOOK: &str = r#"
[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "init"
run = "echo init"
on_done = "done"

[[pipeline.build.step]]
name = "done"
run = "echo done"

[queue.bugs]
vars = ["title"]

[worker.fixer]
source = { queue = "bugs" }
handler = { pipeline = "build" }
concurrency = 2
"#;
```

Steps:
1. Set up runtime with the concurrent runbook
2. Push 3 items to the persisted queue via `QueuePushed` events
3. Start the worker (`WorkerStarted`)
4. Process the resulting `WorkerPollComplete`
5. Assert `active_pipelines.len() == 2` (not 3)
6. Assert 2 `WorkerItemDispatched` events were emitted
7. Assert the 3rd item remains in `Pending` status in `MaterializedState`

#### Test 2: `concurrency_1_still_dispatches_one_item`

Same as above but with `concurrency = 1`. Assert only 1 item dispatched. This is a regression guard.

#### Test 3: `pipeline_completion_triggers_repoll_and_fills_slot`

1. Start worker with concurrency = 2, push 3 items
2. After initial dispatch (2 items), simulate one pipeline completing by processing a `PipelineAdvanced { step: "done" }` event
3. Assert `check_worker_pipeline_complete` triggers a re-poll
4. Assert the 3rd item gets dispatched (active count back to 2)

#### Test 4: `worker_stop_cancels_all_active_pipelines`

1. Start worker with concurrency = 2, dispatch 2 items
2. Process `WorkerStopped` event
3. Assert both pipelines receive cancel events
4. Assert `active_pipelines` is empty
5. Assert worker status is `Stopped`

#### Test 5: `worker_at_capacity_does_not_dispatch`

1. Start worker with concurrency = 2, dispatch 2 items
2. Push another item and trigger `WorkerWake`
3. Process the resulting `WorkerPollComplete` with 1 item
4. Assert no new pipeline is created (already at capacity)
5. Assert `active_pipelines.len()` remains 2

### Phase 5: Verify persisted state recovery with concurrency > 1

**Goal**: Ensure daemon restart correctly restores multiple active pipelines.

#### Test 6: `worker_restart_restores_multiple_active_pipelines`

1. Populate `MaterializedState` with a worker that has 2 active pipelines (simulating pre-restart state)
2. Process `RunbookLoaded` + `WorkerStarted` events (WAL replay)
3. Assert `active_pipelines` contains both pipeline IDs
4. Assert available capacity is 0 (concurrency 2, 2 active)
5. Process a `WorkerPollComplete` with items and assert nothing new is dispatched

This extends the existing `worker_restart_restores_active_pipelines_from_persisted_state` test to cover concurrency > 1.

## Key Implementation Details

### What already works

The core dispatch logic was designed for concurrency > 1 from the start:

- **Slot calculation**: `state.concurrency.saturating_sub(active)` at worker.rs:267
- **Batch dispatch**: `items.iter().take(available_slots)` at worker.rs:294
- **Re-poll check**: `active_pipelines.len() < concurrency` at worker.rs:591
- **Persisted state**: `WorkerRecord.active_pipeline_ids: Vec<String>` stores all active IDs
- **Recovery**: `handle_worker_started` restores all active pipelines from `MaterializedState`

### What needs to change

1. **`handle_worker_stopped`** — must cancel active pipelines (Phase 2). This is the only production code change.

2. **Tests** — all new (Phases 4-5). The existing 4 worker tests only cover concurrency = 1.

### Concurrency model

Workers are single-threaded event processors. "Concurrency" means multiple pipelines can be *active* (in-flight) simultaneously — each pipeline's steps execute asynchronously via effects. The worker doesn't need threads; it just tracks how many pipeline slots are occupied and dispatches more items when slots free up.

### No changes to QueueDef

Concurrency is controlled exclusively at the worker level (`WorkerDef.concurrency`), not the queue level. Multiple workers can watch the same queue with different concurrency settings.

## Verification Plan

1. **Unit tests**: Run `cargo test -p oj-engine` — all new tests from Phase 4-5 plus existing tests pass
2. **Clippy**: `cargo clippy --all-targets --all-features -- -D warnings`
3. **Format**: `cargo fmt --all -- --check`
4. **Full check**: `make check` (includes audit, deny, quench)
5. **Manual smoke test**: Create a runbook with `concurrency = 2`, push 3 items, verify 2 pipelines start immediately and the 3rd starts when one completes
