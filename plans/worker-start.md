# Idempotent Worker Start

## Overview

Make `oj worker start` idempotent: if the worker is already running, wake it instead of failing or duplicating state. Remove `oj worker wake` as a separate command (alias it to `start` for backwards compatibility). Update bugfix.hcl and build.hcl submit steps to auto-start the merge worker when pushing to the merge queue.

## Project Structure

Key files to modify:

```
crates/
├── cli/src/commands/worker.rs      # CLI command definitions
├── daemon/src/listener/workers.rs  # Request handlers (start/wake logic)
├── daemon/src/protocol.rs          # IPC protocol types
├── engine/src/runtime/handlers/
│   ├── mod.rs                      # Event dispatch
│   └── worker.rs                   # Engine-side worker event handling
├── core/src/event.rs               # Event types (WorkerWake removal)
├── storage/src/state.rs            # MaterializedState (worker records)
.oj/runbooks/
├── bugfix.hcl                      # Submit step: add `oj worker start merge`
├── build.hcl                       # Submit step: add `oj worker start merge`
docs/
├── 01-concepts/RUNBOOKS.md         # Update worker docs
├── 02-interface/CLI.md             # Update CLI reference
docs/future/runbooks/
├── reliability.hcl                 # Update `oj worker wake fix` → `oj worker start fix`
├── security.hcl                    # Update `oj worker wake fix` → `oj worker start fix`
```

## Dependencies

No new external dependencies required.

## Implementation Phases

### Phase 1: Make the daemon listener idempotent

The core change: `handle_worker_start` in `crates/daemon/src/listener/workers.rs` must detect whether the worker is already running and emit a wake event instead of a redundant start.

**Current behavior:** `handle_worker_start` always emits `RunbookLoaded` + `WorkerStarted` events, which causes `handle_worker_started` in the engine to overwrite the in-memory `WorkerState` (resetting `active_pipelines` from persisted state, re-triggering initial poll). This is wasteful but not catastrophic — the real problem is it re-initializes state.

**New behavior:** Before emitting `WorkerStarted`, check `MaterializedState::workers` for an existing record with `status == "running"`. If found, emit `WorkerWake` instead and return `Response::WorkerStarted` (same response type for CLI compatibility).

In `crates/daemon/src/listener/workers.rs`, modify `handle_worker_start`:

```rust
pub(super) fn handle_worker_start(
    project_root: &Path,
    worker_name: &str,
    event_bus: &EventBus,
    state: &MaterializedState,  // NEW parameter
) -> Result<Response, ConnectionError> {
    // Check if worker is already running
    if let Some(record) = state.workers.get(worker_name) {
        if record.status == "running" {
            // Already running — just wake it
            let event = Event::WorkerWake {
                worker_name: worker_name.to_string(),
            };
            event_bus.send(event).map_err(|_| ConnectionError::WalError)?;
            return Ok(Response::WorkerStarted {
                worker_name: worker_name.to_string(),
            });
        }
    }

    // ... existing validation and start logic unchanged ...
}
```

Update the call site in `crates/daemon/src/listener/mod.rs` to pass `&state` (the `MaterializedState` snapshot) to `handle_worker_start`. The listener already has access to state for query handling — verify the exact mechanism and thread it through.

**Milestone:** `oj worker start fix` succeeds whether the worker is new or already running. Running it twice no longer duplicates state.

### Phase 2: Remove `WorkerWake` CLI command, alias to `Start`

In `crates/cli/src/commands/worker.rs`:

1. Mark `Wake` as a hidden alias for `Start`:

```rust
#[derive(Subcommand)]
pub enum WorkerCommand {
    /// Start a worker (or wake it if already running)
    Start {
        /// Worker name from runbook
        name: String,
    },
    /// Alias for start (deprecated)
    #[command(hide = true)]
    Wake {
        /// Worker name
        name: String,
    },
    /// List all workers and their status
    List {},
}
```

2. In the `handle` function, make `Wake` dispatch the same `Request::WorkerStart` as `Start`:

```rust
WorkerCommand::Start { name } | WorkerCommand::Wake { name } => {
    let request = Request::WorkerStart {
        project_root: project_root.to_path_buf(),
        worker_name: name.clone(),
    };
    match client.send(&request).await? {
        Response::WorkerStarted { worker_name } => {
            println!("Worker '{}' started", worker_name);
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

**Keep `Request::WorkerWake` and `Event::WorkerWake` in protocol/core** — they are still used internally (by `QueuePushed` handler in queues.rs listener, and by the engine). Only the CLI entry point is removed.

**Milestone:** `oj worker wake fix` still works (hidden) but routes through the same idempotent start path. `oj worker --help` no longer shows `wake`.

### Phase 3: Update runbook submit steps

Update bugfix.hcl and build.hcl submit steps to auto-start the merge worker after pushing to the merge queue. Since start is now idempotent, this is safe to call every time.

**bugfix.hcl** — two changes:

1. The `fix` command currently runs `oj worker wake fix`. Change to `oj worker start fix`:

```hcl
command "fix" {
  args = "<description>"
  run  = <<-SHELL
    wok new bug "${args.description}"
    oj worker start fix
  SHELL
}
```

2. The `submit` step (line 51-61): add `oj worker start merge` after the queue push:

```hcl
step "submit" {
  run = <<-SHELL
    REPO="$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    BRANCH="$(git branch --show-current)"
    git add -A
    git diff --cached --quiet || git commit -m "fix: ${var.bug.title}"
    git -C "$REPO" push origin "$BRANCH"
    oj queue push merges '{"branch": "'"$BRANCH"'", "title": "fix: ${var.bug.title}"}'
    oj worker start merge
  SHELL
  on_done = { step = "done" }
}
```

**build.hcl** — add `oj worker start merge` after queue push in the submit step:

```hcl
step "submit" {
  run = <<-SHELL
    REPO="$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    BRANCH="$(git branch --show-current)"
    git add -A
    git diff --cached --quiet || git commit -m "feat(${var.name}): ${var.instructions}"
    git -C "$REPO" push origin "$BRANCH"
    oj queue push merges '{"branch": "'"$BRANCH"'", "title": "feat(${var.name}): ${var.instructions}"}'
    oj worker start merge
  SHELL
}
```

**Milestone:** Pushing to the merge queue from bugfix/build pipelines automatically starts (or wakes) the merge worker.

### Phase 4: Update docs and future runbooks

1. **`docs/02-interface/CLI.md`** (lines 139-146): Remove `oj worker wake` line, update description:

```
oj worker start <name>               # Start a worker (idempotent; wakes if already running)
oj worker list                       # List all workers
oj worker list -o json               # JSON output
```

Update prose: "Workers poll their source queue and dispatch items to their handler pipeline. `oj worker start` is idempotent — it loads the runbook, validates definitions, and begins the poll-dispatch loop. If the worker is already running, it triggers an immediate poll instead."

2. **`docs/01-concepts/RUNBOOKS.md`** (line 314): Update from "Workers are started via `oj worker start <name>` or woken via `oj worker wake <name>`" to "Workers are started via `oj worker start <name>`. The command is idempotent — if the worker is already running, it wakes it to poll immediately."

3. **`docs/future/runbooks/reliability.hcl`** (line 39): Change `oj worker wake fix` → `oj worker start fix`

4. **`docs/future/runbooks/security.hcl`** (line 35): Change `oj worker wake fix` → `oj worker start fix`

5. **bugfix.hcl header comment** (line 6): Change `oj worker wake fix` reference if present. Current comment says "# File a bug and wake the worker" — update to match new command.

**Milestone:** All documentation and examples reference `oj worker start` consistently.

### Phase 5: Update tests

1. **Engine tests** (`crates/engine/src/runtime_tests/worker.rs`): Add a test that verifies idempotent start behavior — sending `WorkerStarted` when worker already exists in state should wake rather than reinitialize. However, note that the idempotency check happens in the *listener* (Phase 1), not the engine. The engine's `handle_worker_started` is still called only for genuinely new starts. The test should verify the listener behavior.

2. **Listener/integration tests**: If there are existing tests for the listener dispatch, add a test that:
   - Calls `handle_worker_start` with a worker that has `status == "running"` in `MaterializedState`
   - Asserts it emits `Event::WorkerWake` (not `Event::WorkerStarted`)
   - Asserts it returns `Response::WorkerStarted`

3. **Verify existing tests pass**: The `WorkerWake` CLI command still works (hidden alias), so existing tests that use `Request::WorkerWake` should continue to pass without changes.

**Milestone:** `cargo test --all` passes, including new idempotency tests.

## Key Implementation Details

### State check location

The idempotency check must happen in the **daemon listener** (`handle_worker_start`), not the engine. The listener is the synchronous request handler that can return a response to the CLI. The engine processes events asynchronously. By short-circuiting in the listener, we avoid emitting redundant `WorkerStarted` events into the WAL.

### MaterializedState access in the listener

The listener's `handle_request` function in `crates/daemon/src/listener/mod.rs` already has access to a `MaterializedState` snapshot for handling queries. Verify the exact mechanism — it may be passed as a parameter or accessed via a shared reference. Thread it through to `handle_worker_start`.

### Response type consistency

Both the "fresh start" and "already running (wake)" paths return `Response::WorkerStarted { worker_name }`. This keeps the CLI handler simple — it doesn't need to distinguish between the two cases.

### WAL replay safety

During WAL replay on daemon restart, `WorkerStarted` events are replayed to reconstruct `worker_states`. The idempotency check is in the listener (not the engine), so WAL replay is unaffected — the engine still processes `WorkerStarted` events the same way.

### Internal `WorkerWake` event preserved

`Event::WorkerWake` and `Request::WorkerWake` remain in the codebase. They are used internally:
- `QueuePushed` handler in `crates/daemon/src/listener/queues.rs` emits `WorkerWake` for auto-polling
- The listener's `handle_worker_wake` is still called for internal wake events
- The hidden `Wake` CLI command routes through `WorkerStart` now, but the protocol type stays for backwards compatibility with existing WAL entries

## Verification Plan

1. **Unit test**: Listener idempotency — `handle_worker_start` with already-running worker emits `WorkerWake` instead of `WorkerStarted`
2. **Unit test**: Listener fresh start — `handle_worker_start` with no existing worker emits `WorkerStarted` as before
3. **Unit test**: Listener restart of stopped worker — `handle_worker_start` with `status == "stopped"` emits `WorkerStarted` (full restart)
4. **Existing tests**: All existing worker tests in `crates/engine/src/runtime_tests/worker.rs` continue to pass
5. **`make check`**: Full verification suite passes (fmt, clippy, quench, test, build, audit, deny)
6. **Manual smoke test**: `oj worker start merge && oj worker start merge` — second call succeeds without error, worker list shows single entry
