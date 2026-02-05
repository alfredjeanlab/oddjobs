# Concurrency — Future

Proposed changes to the concurrency model described in
[arch/03-concurrency.md](../arch/03-concurrency.md).

## Problem

The event loop blocks for 3-40+ seconds during job creation because
workspace creation, agent spawning, and shell evaluation are awaited inline
within `process_event()`. During these windows:

- Timers cannot fire (liveness checks, idle timeouts, cron schedules)
- Subsequent events queue up (agent state changes, shell completions)
- CLI queries respond normally (state lock is not held across awaits), but
  the system cannot *act* on anything until the current event completes

## Goals

1. No single event blocks the loop for more than ~10ms
2. Timer resolution stays at the configured interval (default 1s)
3. Sequential effect dependencies still work correctly
4. Minimal change to the functional core (pure state machines)

## Proposed Architecture

### Two-tier effect execution

Split effects into **immediate** (executed inline, <10ms) and **deferred**
(spawned as background tasks, emit completion events):

```
┌──────────────────────────────────────────────────────────┐
│                      Event Loop                          │
│                                                          │
│  event = recv()                                          │
│  state.apply(event)                                      │
│  effects = runtime.handle_event(event)                   │
│                                                          │
│  for effect in effects:                                  │
│    match effect:                                         │
│      immediate → execute inline, collect result event    │
│      deferred  → spawn background task                   │
│                                                          │
│  persist result events to WAL                            │
│                                                          │
│  ◄── returns in <10ms ──►                                │
└──────────────────────────────────────────────────────────┘

Background tasks:
  ┌──────────────────────────────────┐
  │  CreateWorkspace task            │
  │  git worktree add ...            │
  │  ──► emit WorkspaceReady/Failed  │
  └──────────────────────────────────┘
  ┌──────────────────────────────────┐
  │  SpawnAgent task                 │
  │  tmux spawn + prompt handling    │
  │  ──► emit SessionCreated/Failed  │
  └──────────────────────────────────┘
  ┌──────────────────────────────────┐
  │  Shell task (already deferred)   │
  │  bash -c "..."                   │
  │  ──► emit ShellExited            │
  └──────────────────────────────────┘
```

### Effect classification

```
Immediate (<10ms, execute inline)     Deferred (spawned, emit result event)
────────────────────────────────────  ──────────────────────────────────────
Emit          state mutation          CreateWorkspace   git worktree
SetTimer      scheduler insert        DeleteWorkspace   git + rm
CancelTimer   scheduler remove        SpawnAgent        tmux + prompts
Notify        fire-and-forget thread   Shell             (already deferred)
                                      PollQueue          (already deferred)
                                      TakeQueueItem     bash subprocess
                                      SendToAgent       tmux + settle
                                      KillAgent         tmux kill
                                      SendToSession     tmux send
                                      KillSession       tmux kill
```

### Sequential dependencies via events

Today, `create_and_start_job()` runs three phases in one async function:

```
Phase 1: Emit JobCreated
Phase 2: execute(CreateWorkspace).await     ← blocks event loop
Phase 3: start_step() → execute(SpawnAgent).await  ← blocks again
```

With deferred effects, each phase becomes an event-driven step:

```
CommandRun event
  → handler emits JobCreated + dispatches CreateWorkspace (deferred)
  → event loop returns immediately

WorkspaceReady event (emitted by background task)
  → handler calls start_step() → dispatches SpawnAgent (deferred)
  → event loop returns immediately

SessionCreated event (emitted by background task)
  → handler sets up watcher, timers
  → event loop returns immediately
```

This is the natural event-driven model. The job state machine already
tracks which step it's on; workspace readiness and agent creation are just
additional state transitions.

**Key change**: `create_and_start_job()` no longer calls
`executor.execute_all(workspace_effects).await` followed by
`start_step().await`. Instead it emits effects and returns. The
`WorkspaceReady` event handler (which already exists for handling workspace
status) triggers `start_step()`. The `WorkspaceFailed` event handler
triggers `fail_job()`.

### Handling pre-creation shell evaluation

The current `create_and_start_job()` also evaluates `workspace.ref` and
`locals` by running shell subprocesses inline. These block the event loop for
50-500ms each.

**Option A: Evaluate in the deferred task.** Move `workspace.ref` and `locals`
evaluation into the `CreateWorkspace` background task, passing the unevaluated
templates as part of the effect. The task evaluates them, then creates the
workspace. Result events carry the resolved values back to update job
vars.

**Option B: Add a PrepareWorkspace effect.** Create a new deferred effect that
evaluates shell expressions and emits a `WorkspacePrepared` event with
resolved values. The handler for `WorkspacePrepared` then dispatches the
actual `CreateWorkspace` effect. This adds one more event hop but keeps
workspace creation simple.

**Option C: Accept the latency.** Shell expression evaluation is typically
<500ms. If the goal is "no event blocks for >10ms" this still violates the
contract, but the practical impact is low compared to multi-second git
operations. Document the trade-off and defer.

Option A is simplest if workspace creation already needs the invoke directory
and environment context.

### Effect failure handling

Deferred effects need a failure path. Today, `execute_all()` returns
`Result<Vec<Event>, ExecuteError>` and the caller handles errors inline. With
deferred effects, failures are asynchronous:

- Background task catches the error
- Emits a failure event (e.g., `WorkspaceFailed`, `AgentSpawnFailed`)
- The event loop processes the failure event and fails the job

This pattern already exists for `Shell` effects: the background task catches
errors and emits `ShellExited` with a non-zero exit code. Workspace and agent
effects should follow the same pattern.

### SendToAgent settle time

`SendToAgent` blocks for up to 2.2 seconds due to sleep-based settle timing
(2x Esc key pause + text settle at 1ms/char, capped at 2s). This is
inherently timing-dependent and cannot be made event-driven.

Options:
- Move to deferred execution. The send doesn't produce a result event that
  the job depends on — the agent watcher detects when the agent starts
  working. Fire-and-forget semantics are safe here.
- Reduce settle time. The 2s cap is conservative; measure whether shorter
  times cause issues in practice.

## Implementation Sketch

### Phase 1: Deferred workspace and agent effects

1. Add `WorkspaceFailed` event to the core event enum (if not present)
2. Add `AgentSpawnFailed` event to the core event enum
3. Move `CreateWorkspace` execution into a `tokio::spawn` task in the executor,
   following the `Shell` effect pattern
4. Move `SpawnAgent` execution into a `tokio::spawn` task
5. Refactor `create_and_start_job()` to return after emitting effects
   instead of awaiting them
6. Add `WorkspaceReady` → `start_step()` handler to the runtime
7. Add `WorkspaceFailed` → `fail_job()` handler
8. Add `AgentSpawnFailed` → `fail_job()` handler

### Phase 2: Deferred remaining I/O effects

1. Move `DeleteWorkspace` to deferred (emit `WorkspaceDeleted`)
2. Move `SendToAgent` to deferred (fire-and-forget, no result event)
3. Move `KillAgent`, `KillSession`, `SendToSession` to deferred
4. Move `TakeQueueItem` to deferred (same pattern as `Shell`/`PollQueue`)

### Phase 3: IPC handler timeouts

1. Wrap all subprocess calls in listener handlers with `tokio::time::timeout`
2. `PeekSession` (tmux capture): 5s timeout
3. `WorkspacePrune` (git operations): 30s timeout per workspace
4. `AgentResume` (tmux kills): 5s timeout per session

## Non-goals

- **Parallel effect execution within a batch.** Effects within a single
  `execute_all()` call could run concurrently, but the complexity isn't
  justified. Sequential deferred effects with event-driven chaining achieves
  the same throughput without coordination hazards.

- **Async-aware mutexes.** The current `parking_lot::Mutex` usage is correct:
  locks are never held across `.await` points and critical sections are
  microsecond-scale. Switching to `tokio::sync::Mutex` would add overhead
  without benefit.

- **`spawn_blocking` for file I/O.** The blocking file I/O in the codebase
  (WAL flush, snapshot save, log reads) is brief and infrequent. Adding
  `spawn_blocking` everywhere would increase code complexity for marginal
  benefit on the current workload.

## See Also

- [arch/03-concurrency.md](../arch/03-concurrency.md) — Current concurrency model
- [arch/02-effects.md](../arch/02-effects.md) — Effect types and execution
- [arch/01-daemon.md](../arch/01-daemon.md) — Event loop and daemon lifecycle
