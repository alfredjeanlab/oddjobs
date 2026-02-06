# Adapters

Adapters abstract external system I/O, enabling comprehensive testing without real tmux/git/etc.

## Pattern

```
State Machine → Effect → Executor → TracedAdapter → Adapter → subprocess
                                         ↓
                                   FakeAdapter (tests)
```

State machines are pure. Adapters handle all I/O. Tests use fakes.

## What Gets an Adapter

Adapters wrap **external tools and abstractions** with predictable behavior:

| Tool/Concept | Adapter | Why |
|--------------|---------|-----|
| tmux | `SessionAdapter` | Low-level terminal session management |
| Claude Code | `AgentAdapter` | Agent lifecycle, prompts, state detection |
| channels | `NotifyAdapter` | Notifications to external channels |

## Adapter Traits

| Trait | Wraps | Key Methods |
|-------|-------|-------------|
| `SessionAdapter` | tmux | spawn, send, send_literal, send_enter, kill, is_alive, capture_output, is_process_running, get_exit_code, configure |
| `AgentAdapter` | Claude Code | spawn, reconnect, send, get_state, kill, session_log_size |
| `NotifyAdapter` | desktop | notify |

## AgentAdapter

Manages AI agent lifecycle and behavior. This is a higher-level abstraction than `SessionAdapter` - it encapsulates agent-specific concerns like prompt handling and state detection.

```rust
#[async_trait]
pub trait AgentAdapter: Clone + Send + Sync + 'static {
    /// Spawn a new agent
    ///
    /// Prepares the workspace, spawns the session, and starts a background
    /// watcher that emits `AgentStateChanged` events via `event_tx`.
    async fn spawn(
        &self,
        config: AgentSpawnConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError>;

    /// Send input to an agent (nudge, follow-up)
    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError>;

    /// Kill an agent (stops session and background watcher)
    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError>;

    /// Reconnect to an existing agent session (e.g. after daemon restart)
    async fn reconnect(
        &self,
        config: AgentReconnectConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError>;

    /// Get current agent state from session log (point-in-time check)
    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError>;

    /// Get the current size of the agent's session log file in bytes.
    /// Used by the idle grace timer to detect activity during the grace period.
    async fn session_log_size(&self, agent_id: &AgentId) -> Option<u64>;
}

pub enum AgentState {
    /// Agent is actively working (processing or running tools)
    Working,
    /// Agent finished and is waiting for user input
    WaitingForInput,
    /// Agent encountered a failure
    Failed(AgentError),
    /// Agent process exited
    Exited { exit_code: Option<i32> },
    /// Agent session is gone (process terminated unexpectedly)
    SessionGone,
}
```

`AgentSpawnConfig` bundles spawn parameters: `agent_id`, `agent_name`, `command`, `env`, `workspace_path`, `cwd`, `prompt`, `job_name`, `job_id`, `project_root`.

`AgentReconnectConfig` bundles reconnect parameters: `agent_id`, `session_id`, `workspace_path`, `process_name`.

Liveness is detected via the background watcher (file-watching + periodic process checks), not a separate `is_alive()` method.

**Production** (`ClaudeAgentAdapter<S: SessionAdapter>`): Generic over the session adapter. Wraps `SessionAdapter` with Claude-specific behavior:
- Auto-accepts trust/permission prompts after spawn
- Parses Claude's JSONL session log for state detection
- Handles Claude-specific error patterns

**Fake** (`FakeAgentAdapter`): In-memory state, configurable responses, records all calls.

### Why a Separate Adapter?

`SessionAdapter` is low-level - it knows about tmux but nothing about agents. Agent behavior includes:

- **Startup prompts**: Trust dialogs, permission requests that need acknowledgement
- **State detection**: Parsing session logs for `stop_reason`, error patterns
- **Agent-specific protocols**: How to detect idle vs working, how to nudge

Keeping this in `AgentAdapter` means:
- `SessionAdapter` stays simple (just tmux operations)
- Agent behavior is testable via `FakeAgentAdapter`
- Different agent types can have their own implementations

## SessionAdapter

Low-level terminal session management via tmux.

```rust
#[async_trait]
pub trait SessionAdapter: Clone + Send + Sync + 'static {
    async fn spawn(
        &self,
        name: &str,
        cwd: &Path,
        cmd: &str,
        env: &[(String, String)],
    ) -> Result<String, SessionError>;
    async fn send(&self, id: &str, input: &str) -> Result<(), SessionError>;
    async fn send_literal(&self, id: &str, text: &str) -> Result<(), SessionError>;
    async fn send_enter(&self, id: &str) -> Result<(), SessionError>;
    async fn kill(&self, id: &str) -> Result<(), SessionError>;
    async fn is_alive(&self, id: &str) -> Result<bool, SessionError>;
    async fn capture_output(&self, id: &str, lines: u32) -> Result<String, SessionError>;
    async fn is_process_running(&self, id: &str, pattern: &str) -> Result<bool, SessionError>;
    /// Get the exit code of the pane's process (if available).
    /// Returns `None` if the pane is still running or the exit code is unavailable.
    async fn get_exit_code(&self, id: &str) -> Result<Option<i32>, SessionError>;
    /// Apply configuration to an existing session (styling, status bar, etc.)
    async fn configure(&self, id: &str, config: &serde_json::Value) -> Result<(), SessionError>;
}
```

**Production** (`TmuxAdapter`): Shells out to tmux commands.

**Fake** (`FakeSessionAdapter`): In-memory state, records all calls.

## NotifyAdapter

Sends desktop notifications.

```rust
#[async_trait]
pub trait NotifyAdapter: Clone + Send + Sync + 'static {
    /// Send a notification with a title and message body
    async fn notify(&self, title: &str, message: &str) -> Result<(), NotifyError>;
}
```

**Production** (`DesktopNotifyAdapter`): Sends native desktop notifications via notify-rust.

**Fake** (`FakeNotifyAdapter`): Records notifications for test assertions.

## Traced Wrappers

Adapters are wrapped with instrumentation for observability:

```rust
// At construction (in daemon lifecycle)
let session_adapter = TracedSession::new(TmuxAdapter::new());
let agent_adapter = TracedAgent::new(
    ClaudeAgentAdapter::new(session_adapter.clone()).with_log_entry_tx(log_entry_tx),
);
```

Traced wrappers provide **generic** observability:
- Entry/exit logging with operation-specific fields
- Timing metrics (`elapsed_ms`) on every call
- Consistent error logging with context

Production adapters retain **implementation-specific** logging:
- `TmuxAdapter`: Warns when killing existing session before spawn
- `ClaudeAgentAdapter`: Logs prompt detection and auto-acknowledgement
- Other operational details that the generic wrapper can't know about

This layering keeps observability consistent while preserving useful implementation details.

## Testing

Fakes enable:
- **Deterministic tests**: No real tmux/git/claude needed
- **Call verification**: Assert exactly what operations were attempted
- **Error injection**: `set_spawn_fails(true)` to test error paths
- **State simulation**: Pre-populate sessions, agent states

Integration tests with real adapters use `#[ignore]` and run separately.

For agent integration tests, use [claudeless](https://github.com/anthropics/claudeless) - a CLI simulator that emulates Claude's interface, TUI, hooks, and permissions without API costs. The `ClaudeAgentAdapter` works with both real Claude and claudeless.
