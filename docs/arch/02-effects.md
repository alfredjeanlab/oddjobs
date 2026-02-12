# Effects

All side effects are represented as data, not function calls. The functional core returns effects; the imperative shell executes them.

## Effect Types

```rust
pub enum Effect {
    Emit { event },              // Persist + apply event
    SpawnAgent { .. },           // Launch agent via coop
    SendToAgent { .. },          // Send text input to agent
    RespondToAgent { .. },       // Structured prompt response
    KillAgent { .. },            // Terminate agent
    CreateWorkspace { .. },      // Create folder or git worktree
    DeleteWorkspace { .. },      // Remove workspace
    SetTimer { id, duration },   // Schedule future event
    CancelTimer { id },          // Cancel scheduled timer
    Shell { .. },                // Run shell command
    Notify { title, message },   // Desktop notification
    PollQueue { .. },            // List external queue items
    TakeQueueItem { .. },        // Claim external queue item
}
```

## Why Effects as Data

Effects as data enables:

1. **Testability** - Assert on effects without executing I/O
2. **Logging** - Inspect effects before execution
3. **Dry-run** - Validate without side effects
4. **Replay** - Debug by replaying effect sequences

## Execution

The event loop processes events through the runtime, which produces effects via the executor. Result events are fed back iteratively:

```
loop {
    event = next_event()
    result_events = runtime.handle_event(event)
    for result_event in result_events {
        persist(result_event)
        pending.push(result_event)
    }
}
```

The runtime's `handle_event` dispatches to handler methods that build effects and execute them via the `Executor`. Effects are split into **immediate** (executed inline, <10ms) and **deferred** (spawned as background tasks that emit completion events back into the event loop):

| Category | Effects | Mechanism |
|----------|---------|-----------|
| Immediate | Emit, SetTimer, CancelTimer, Notify | Inline (<10ms) |
| Deferred | SpawnAgent, SendToAgent, RespondToAgent, KillAgent | AgentAdapter (background task) |
| Deferred | CreateWorkspace, DeleteWorkspace | Filesystem / git subprocess |
| Deferred | Shell, PollQueue, TakeQueueItem | tokio subprocess |

Deferred effects return immediately after spawning a `tokio::spawn` background task. The background task executes the I/O and emits a result event (e.g., `WorkspaceReady`, `AgentSpawned`) back through the event bus. This keeps event loop iterations under ~10ms.

## Instrumentation

`Effect` provides `name()` and `fields()` methods for consistent observability. The executor wraps all effect execution with tracing spans, entry/exit logging, and elapsed time metrics.

## Timer Effects

Timers schedule future events:

```rust
Effect::SetTimer {
    id: TimerId::liveness(&job_id),
    duration: Duration::from_secs(30),
}
// Later, scheduler delivers:
Event::TimerStart { id: TimerId }
```

Timer IDs use structured constructors on `TimerId`. Owner-based timers accept `impl Into<OwnerId>`, so they work with both `JobId` and `CrewId`:
- `TimerId::liveness(owner)` — Liveness check timer
- `TimerId::exit_deferred(owner)` — Deferred exit handling
- `TimerId::cooldown(owner, trigger, chain_pos)` — Cooldown between action attempts
- `TimerId::queue_retry(queue_name, item_id)` — Queue item retry delay
- `TimerId::cron(cron_name, project)` — Cron interval timer
- `TimerId::queue_poll(worker_name, project)` — External queue poll interval
