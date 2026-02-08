# Effects

All side effects are represented as data, not function calls. The functional core returns effects; the imperative shell executes them.

## Effect Types

```rust
pub enum Effect {
    // Event emission
    Emit { event: Event },

    // Agent-level effects (preferred for job operations)
    SpawnAgent {
        agent_id: AgentId,
        agent_name: String,
        owner: OwnerId,
        .. // and so on
    },
    SendToAgent { agent_id: AgentId, input: String },
    KillAgent { agent_id: AgentId },

    // Session-level effects (low-level, used by AgentAdapter)
    SendToSession { session_id: SessionId, input: String },
    KillSession { session_id: SessionId },

    // Workspace effects
    CreateWorkspace {
        workspace_id: WorkspaceId,
        path: PathBuf,
        owner: Option<OwnerId>,
        .. // and so on
    },
    DeleteWorkspace { workspace_id: WorkspaceId },

    // Timer effects
    SetTimer { id: TimerId, duration: Duration },
    CancelTimer { id: TimerId },

    // Shell effects
    Shell {
        owner: Option<OwnerId>,   // Job or agent_run
        step: String,
        command: String,
        cwd: PathBuf,
        env: HashMap<String, String>,
    },

    // Notification effects
    Notify { title: String, message: String },

    // Worker/queue effects
    PollQueue { worker_name: String, list_command: String, cwd: PathBuf },
    TakeQueueItem {
        worker_name: String,
        take_command: String,
        cwd: PathBuf,
        item_id: String,       // ID of the item being taken
        item: serde_json::Value,  // Full item data for job creation
    },
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

| Effect | Execution | Adapter / Mechanism |
|--------|-----------|---------------------|
| Emit | Immediate | MaterializedState (apply + WAL) |
| SetTimer, CancelTimer | Immediate | Scheduler |
| Notify | Immediate | notify_rust (fire-and-forget thread) |
| CreateWorkspace | Deferred | Filesystem / git subprocess → `WorkspaceReady` / `WorkspaceFailed` |
| DeleteWorkspace | Deferred | Filesystem / git subprocess → `WorkspaceDeleted` |
| SpawnAgent | Deferred | AgentAdapter → `SessionCreated` / `AgentSpawnFailed` |
| SendToAgent | Deferred | AgentAdapter (fire-and-forget, no result event) |
| KillAgent | Deferred | AgentAdapter (fire-and-forget) |
| SendToSession | Deferred | SessionAdapter (fire-and-forget) |
| KillSession | Deferred | SessionAdapter (fire-and-forget) |
| Shell | Deferred | tokio subprocess → `ShellExited` |
| PollQueue | Deferred | tokio subprocess |
| TakeQueueItem | Deferred | tokio subprocess |

Deferred effects follow the same pattern: the executor spawns a `tokio::spawn` background task and returns immediately. The background task executes the I/O and emits a result event (e.g., `WorkspaceReady`, `SessionCreated`) back through the event bus. This keeps event loop iterations under ~10ms.

### Agent vs Session Effects

Use **Agent effects** (`SpawnAgent`, `SendToAgent`, `KillAgent`) for AI agent invocations. The `AgentAdapter`:
- Handles startup prompts (trust dialogs, permissions)
- Parses session logs for state detection
- Provides agent-level abstractions

Use **Session effects** (`SendToSession`, `KillSession`) for low-level terminal operations where agent behavior isn't needed.

## Instrumentation

`Effect` provides `name()` and `fields()` methods for consistent observability.
The executor wraps all effect execution with tracing:

```rust
pub async fn execute(&self, effect: Effect) -> Result<Option<Event>, ExecuteError> {
    let op_name = effect.name();
    let span = tracing::info_span!("effect", effect = op_name);
    let _guard = span.enter();

    tracing::info!(fields = ?effect.fields(), "executing");

    let start = std::time::Instant::now();
    let result = self.execute_inner(effect).await;
    let elapsed = start.elapsed();

    // Log completion or error with elapsed time
}
```

This provides:
- Entry logging with effect-specific fields
- Timing metrics on every operation
- Consistent error logging with context

## Timer Effects

Timers schedule future events:

```rust
// State machine returns timer effect
Effect::SetTimer {
    id: TimerId::liveness(&job_id),
    duration: Duration::from_secs(30),
}

// Later, scheduler delivers timer event
Event::TimerStart { id: TimerId }
```

Timer IDs use structured constructors on `TimerId`:
- `TimerId::liveness(job_id)` -- `"liveness:{job_id}"`
- `TimerId::exit_deferred(job_id)` -- `"exit-deferred:{job_id}"`
- `TimerId::cooldown(job_id, trigger, chain_pos)` -- `"cooldown:{job_id}:{trigger}:{chain_pos}"`
- `TimerId::idle_grace(job_id)` -- `"idle-grace:{job_id}"`
- `TimerId::queue_retry(queue_name, item_id)` -- `"queue-retry:{queue_name}:{item_id}"`
- `TimerId::cron(cron_name, namespace)` -- `"cron:{scoped_name}"`
- `TimerId::queue_poll(worker_name, namespace)` -- `"queue-poll:{scoped_name}"`

Agent run variants mirror the job variants with an `ar:` infix:
- `TimerId::liveness_agent_run(id)` -- `"liveness:ar:{id}"`
- `TimerId::exit_deferred_agent_run(id)` -- `"exit-deferred:ar:{id}"`
- `TimerId::cooldown_agent_run(id, trigger, chain_pos)` -- `"cooldown:ar:{id}:{trigger}:{chain_pos}"`
- `TimerId::idle_grace_agent_run(id)` -- `"idle-grace:ar:{id}"`

Unified constructors dispatch by `OwnerId`:
- `TimerId::owner_liveness(owner)`, `owner_exit_deferred(owner)`, `owner_cooldown(owner, ..)`, `owner_idle_grace(owner)`
