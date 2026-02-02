# Effects

All side effects are represented as data, not function calls. The functional core returns effects; the imperative shell executes them.

## Effect Types

```rust
pub enum Effect {
    // Event emission
    Emit { event: Event },

    // Agent-level effects (preferred for pipeline operations)
    SpawnAgent {
        agent_id: AgentId,
        agent_name: String,
        pipeline_id: PipelineId,
        workspace_path: PathBuf,
        input: HashMap<String, String>,
        command: String,
        env: Vec<(String, String)>,
        cwd: Option<PathBuf>,
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
        owner: Option<String>,
        mode: Option<String>,
    },
    DeleteWorkspace { workspace_id: WorkspaceId },

    // Timer effects
    SetTimer { id: TimerId, duration: Duration },
    CancelTimer { id: TimerId },

    // Shell effects
    Shell {
        pipeline_id: PipelineId,
        step: String,
        command: String,
        cwd: PathBuf,
        env: HashMap<String, String>,
    },

    // Notification effects
    Notify { title: String, message: String },

    // Worker/queue effects
    PollQueue { worker_name: String, list_command: String, cwd: PathBuf },
    TakeQueueItem { worker_name: String, take_command: String, cwd: PathBuf },
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

The runtime's `handle_event` dispatches to handler methods that build effects and execute them via the `Executor`. The executor runs effects using adapters and returns any resulting events (e.g., `SpawnAgent` returns `SessionCreated`, `CreateWorkspace` returns `WorkspaceReady`).

Effects are executed via adapters:

| Effect | Adapter |
|--------|---------|
| SpawnAgent, SendToAgent, KillAgent | AgentAdapter |
| SendToSession, KillSession | SessionAdapter |
| CreateWorkspace, DeleteWorkspace | MaterializedState + filesystem |
| Shell | tokio subprocess (async, emits ShellExited event) |
| Emit | MaterializedState (apply + WAL) |
| SetTimer, CancelTimer | Scheduler |
| Notify | notify_rust (fire-and-forget background thread) |
| PollQueue, TakeQueueItem | tokio subprocess |

### Agent vs Session Effects

Use **Agent effects** (`SpawnAgent`, `SendToAgent`, `KillAgent`) for AI agent invocations. The `AgentAdapter`:
- Handles startup prompts (trust dialogs, permissions)
- Parses session logs for state detection
- Provides agent-level abstractions

Use **Session effects** (`SendToSession`, `KillSession`) for low-level terminal operations where agent behavior isn't needed.

## Instrumentation

Effects implement `TracedEffect` for consistent observability:

```rust
pub trait TracedEffect {
    /// Effect name for log spans (e.g., "spawn_agent", "shell")
    fn name(&self) -> &'static str;

    /// Key-value pairs for structured logging
    fn fields(&self) -> Vec<(&'static str, String)>;
}
```

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
    id: TimerId::liveness(&pipeline_id),
    duration: Duration::from_secs(30),
}

// Later, scheduler delivers timer event
Event::TimerStart { id: TimerId }
```

Timer IDs use structured constructors on `TimerId`:
- `TimerId::liveness(pipeline_id)` -- `"liveness:{pipeline_id}"`
- `TimerId::exit_deferred(pipeline_id)` -- `"exit-deferred:{pipeline_id}"`
- `TimerId::cooldown(pipeline_id, trigger, chain_pos)` -- `"cooldown:{pipeline_id}:{trigger}:{chain_pos}"`
