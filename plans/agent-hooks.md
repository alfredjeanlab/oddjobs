# Agent Hooks: Instant State Detection via Claude Code Notification Hooks

## Overview

Replace the 3-minute log-based idle timeout with instant agent state detection by injecting Claude Code **Notification hooks** into agent settings. When Claude enters an idle prompt or permission prompt, the hook fires `oj emit agent:idle` or `oj emit agent:prompt` immediately, delivering sub-second state detection. The log-based watcher remains as a defense-in-depth fallback.

## Project Structure

```
crates/
├── cli/src/commands/emit.rs        # Add agent:idle and agent:prompt subcommands
├── core/src/event.rs               # Add AgentIdle and AgentPrompt events + PromptType enum
├── engine/
│   ├── src/workspace.rs            # Inject Notification hooks into settings
│   ├── src/workspace_tests.rs      # Test hook injection
│   ├── src/monitor.rs              # Add MonitorState::Prompting variant
│   ├── src/monitor_tests.rs        # Test MonitorState::Prompting
│   └── src/runtime/
│       ├── handlers/mod.rs         # Route AgentIdle and AgentPrompt events
│       ├── handlers/agent.rs       # Handle new events with precedence logic
│       └── monitor.rs              # handle_monitor_state for Prompting
├── runbook/src/agent.rs            # Add on_prompt field to AgentDef
└── daemon/src/lifecycle.rs         # (no changes needed - events flow through existing process_event)
```

## Dependencies

No new external dependencies. Uses existing:
- `serde_json` for hook injection
- `clap` for CLI subcommands
- `oj_core` event infrastructure
- Existing WAL/EventBus pipeline for durability

## Implementation Phases

### Phase 1: Core Types — Events and PromptType Enum

Add the new event variants and prompt type classification.

**`crates/core/src/event.rs`** — Add `PromptType` enum and two new events:

```rust
/// Type of prompt the agent is showing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptType {
    Permission,
    Idle,
    PlanApproval,
    Question,
    Other,
}

// In the Event enum:

#[serde(rename = "agent:idle")]
AgentIdle { agent_id: AgentId },

#[serde(rename = "agent:prompt")]
AgentPrompt {
    agent_id: AgentId,
    #[serde(default = "default_prompt_type")]
    prompt_type: PromptType,
},
```

Update `Event::name()`, `Event::log_summary()`, and the no-op match arm in `handle_event` (or the active handler arm — see Phase 5).

Add `PromptType` to the `oj_core` public API (re-export from `lib.rs`).

**Milestone:** `cargo test -p oj-core` passes; new event variants serialize/deserialize correctly.

---

### Phase 2: Inject Notification Hooks into Agent Settings

Extend `inject_hooks()` in `crates/engine/src/workspace.rs` to inject `Notification` hooks alongside the existing `Stop` hook.

Claude Code Notification hooks fire on specific matchers. The two matchers we need:
- `idle_prompt` — fires when Claude is idle/waiting for user input
- `permission_prompt` — fires when Claude asks for permission to use a tool

```rust
// In inject_hooks(), after the Stop hook insertion:

let idle_hook_entry = json!({
    "matcher": "idle_prompt",
    "hooks": [{
        "type": "command",
        "command": format!("oj emit agent:idle --agent {}", agent_id)
    }]
});

let permission_hook_entry = json!({
    "matcher": "permission_prompt",
    "hooks": [{
        "type": "command",
        "command": format!("oj emit agent:prompt --agent {} --type permission", agent_id)
    }]
});

hooks_obj.insert("Notification".to_string(), json!([
    idle_hook_entry,
    permission_hook_entry,
]));
```

**Key detail:** These hooks must NOT block Claude. The `oj emit` commands are fire-and-forget — they send an event to the daemon and exit. Claude Code runs Notification hooks asynchronously by default, so no special `async:true` flag is needed (that's implicit for Notification hooks).

**`crates/engine/src/workspace_tests.rs`** — Add test verifying:
- Notification hooks are present in generated settings
- Both `idle_prompt` and `permission_prompt` matchers are configured
- Commands contain the correct agent ID

**Milestone:** `cargo test -p oj-engine` passes; settings files contain Notification hooks.

---

### Phase 3: CLI Emit Commands

Add `agent:idle` and `agent:prompt` subcommands to `crates/cli/src/commands/emit.rs`.

```rust
#[derive(Subcommand)]
pub enum EmitCommand {
    /// Signal agent completion to the daemon
    #[command(name = "agent:signal")]
    AgentDone { /* existing */ },

    /// Report agent idle (from Notification hook)
    #[command(name = "agent:idle")]
    AgentIdle {
        #[arg(long = "agent")]
        agent_id: String,
    },

    /// Report agent prompt (from Notification hook)
    #[command(name = "agent:prompt")]
    AgentPrompt {
        #[arg(long = "agent")]
        agent_id: String,
        #[arg(long = "type", default_value = "other")]
        prompt_type: String,
    },
}
```

Handler creates `Event::AgentIdle` or `Event::AgentPrompt` and calls `client.emit_event()`.

Parse `--type` string to `PromptType`: "permission" → `Permission`, "idle" → `Idle`, "plan_approval" → `PlanApproval`, "question" → `Question`, _ → `Other`.

**Milestone:** `oj emit agent:idle --agent test123` sends event to daemon; `oj emit agent:prompt --agent test123 --type permission` sends event.

---

### Phase 4: MonitorState::Prompting and on_prompt in AgentDef

**`crates/engine/src/monitor.rs`** — Add `Prompting` variant:

```rust
pub enum MonitorState {
    Working,
    WaitingForInput,
    Prompting { prompt_type: PromptType },
    Failed { message: String, error_type: Option<ErrorType> },
    Exited,
    Gone,
}
```

**`crates/runbook/src/agent.rs`** — Add `on_prompt` field to `AgentDef`:

```rust
pub struct AgentDef {
    // ... existing fields ...

    /// What to do when agent shows a permission/approval prompt
    #[serde(default = "default_on_prompt")]
    pub on_prompt: ActionConfig,
}

fn default_on_prompt() -> ActionConfig {
    ActionConfig::Simple(AgentAction::Escalate)
}
```

Update `Default for AgentDef` to include `on_prompt: default_on_prompt()`.

Add `ActionTrigger::OnPrompt` variant:
```rust
pub enum ActionTrigger {
    OnIdle,
    OnDead,
    OnError,
    OnPrompt,
}
```

Valid actions for `OnPrompt`: `Escalate`, `Done`, `Fail`, `Gate` (same as `OnIdle` minus `Nudge` — nudging a permission prompt doesn't make sense).

**`crates/engine/src/runtime/monitor.rs`** — Add `Prompting` arm to `handle_monitor_state`:

```rust
MonitorState::Prompting { prompt_type } => {
    tracing::info!(
        pipeline_id = %pipeline.id,
        prompt_type = ?prompt_type,
        "agent prompting (on_prompt)"
    );
    self.logger.append(
        &pipeline.id,
        &pipeline.step,
        &format!("agent prompt: {:?}", prompt_type),
    );
    (&agent_def.on_prompt, "prompt")
}
```

**Milestone:** `cargo test -p oj-runbook -p oj-engine` passes; HCL/TOML with `on_prompt = "escalate"` parses correctly.

---

### Phase 5: Event Routing and Precedence

Wire the new events through `crates/engine/src/runtime/handlers/mod.rs` and implement precedence logic in `crates/engine/src/runtime/handlers/agent.rs`.

**Event routing in `handlers/mod.rs`:**

```rust
Event::AgentIdle { agent_id } => {
    result_events.extend(
        self.handle_agent_idle_hook(agent_id).await?
    );
}

Event::AgentPrompt { agent_id, prompt_type } => {
    result_events.extend(
        self.handle_agent_prompt_hook(agent_id, prompt_type).await?
    );
}
```

**Precedence logic in `handlers/agent.rs`:**

The precedence order (highest to lowest):
1. **`agent:signal complete`** — always wins, advances pipeline immediately
2. **`agent:prompt`** — agent is at a permission/approval prompt
3. **`agent:idle`** (from hook) — agent returned to idle prompt
4. **Log-based idle** (existing `AgentWaiting`) — fallback from file watcher

Implementation approach — use the pipeline's `step_status` and `agent_signal` fields to enforce precedence:

```rust
/// Handle agent:idle from Notification hook
pub(crate) async fn handle_agent_idle_hook(
    &self,
    agent_id: &AgentId,
) -> Result<Vec<Event>, RuntimeError> {
    let Some(pipeline_id) = self.agent_pipelines.lock().get(agent_id).cloned() else {
        return Ok(vec![]);
    };
    let pipeline = self.require_pipeline(&pipeline_id)?;

    // If pipeline already advanced or has a signal, ignore
    if pipeline.is_terminal() || pipeline.agent_signal.is_some() {
        return Ok(vec![]);
    }

    // Stale agent check (same as handle_agent_state_changed)
    let current_agent_id = pipeline.step_history.iter()
        .rfind(|r| r.name == pipeline.step)
        .and_then(|r| r.agent_id.as_deref());
    if current_agent_id != Some(agent_id.as_str()) {
        return Ok(vec![]);
    }

    let runbook = self.cached_runbook(&pipeline.runbook_hash)?;
    let agent_def = monitor::get_agent_def(&runbook, &pipeline)?.clone();
    self.handle_monitor_state(&pipeline, &agent_def, MonitorState::WaitingForInput).await
}

/// Handle agent:prompt from Notification hook
pub(crate) async fn handle_agent_prompt_hook(
    &self,
    agent_id: &AgentId,
    prompt_type: &PromptType,
) -> Result<Vec<Event>, RuntimeError> {
    // Same guard logic as handle_agent_idle_hook...

    let runbook = self.cached_runbook(&pipeline.runbook_hash)?;
    let agent_def = monitor::get_agent_def(&runbook, &pipeline)?.clone();
    self.handle_monitor_state(
        &pipeline,
        &agent_def,
        MonitorState::Prompting { prompt_type: prompt_type.clone() },
    ).await
}
```

**Existing `AgentWaiting` (log-based) handling** — no changes needed. The existing `handle_agent_state_changed` → `handle_monitor_state(WaitingForInput)` path continues to work. If the hook-based `agent:idle` already fired and triggered `on_idle` (e.g., `done` → pipeline advanced), then `handle_agent_state_changed` will see `pipeline.is_terminal()` and bail out. The attempt-tracking mechanism (`increment_action_attempt`) also prevents double-firing when attempts=1.

**Milestone:** Full event flow works end-to-end; `agent:idle` hook triggers `on_idle` instantly; `agent:prompt` triggers `on_prompt` (escalate by default).

---

### Phase 6: Tests and Verification

Add unit tests with `FakeAdapters` covering:

1. **Hook injection test** (`workspace_tests.rs`):
   - Settings contain `Notification` key with two hook entries
   - `idle_prompt` matcher with correct `oj emit agent:idle` command
   - `permission_prompt` matcher with correct `oj emit agent:prompt` command

2. **Event precedence tests** (`handlers/agent.rs` or new test file):
   - `agent:signal complete` while `agent:idle` pending → pipeline advances (signal wins)
   - `agent:idle` after pipeline already advanced → no-op
   - `agent:prompt` fires `on_prompt` action (default: escalate)
   - `agent:idle` (hook) followed by `AgentWaiting` (log) → second is no-op (attempts exhausted)
   - Stale agent_id events are dropped

3. **on_prompt default escalate test** (`agent_tests.rs` or `monitor_tests.rs`):
   - AgentDef with no `on_prompt` field → defaults to escalate
   - AgentDef with `on_prompt = "done"` → parses correctly

4. **Event serialization tests** (`event_tests.rs`):
   - `AgentIdle` round-trips through JSON
   - `AgentPrompt` with each `PromptType` variant round-trips
   - Unknown `prompt_type` deserializes to `Other`

**Milestone:** `make check` passes (fmt, clippy, tests, build, audit, deny).

## Key Implementation Details

### Claude Code Hook Format

Claude Code settings support hooks at specific lifecycle events. The `Notification` hook fires whenever Claude displays a notification to the user (idle prompt, permission request, etc.). Each hook entry has a `matcher` field to filter which notifications trigger it:

```json
{
  "hooks": {
    "Stop": [{ "matcher": "", "hooks": [{ "type": "command", "command": "..." }] }],
    "Notification": [
      { "matcher": "idle_prompt", "hooks": [{ "type": "command", "command": "..." }] },
      { "matcher": "permission_prompt", "hooks": [{ "type": "command", "command": "..." }] }
    ]
  }
}
```

### Event Precedence

The system must handle multiple sources of truth for agent state. Precedence prevents conflicting actions:

```
agent:signal complete  →  ALWAYS advance (authoritative, explicit)
agent:prompt           →  fire on_prompt (hook-based, instant)
agent:idle (hook)      →  fire on_idle (hook-based, instant)
AgentWaiting (log)     →  fire on_idle (log-based, fallback ~3min delay)
```

Natural guards prevent conflicts:
- `pipeline.is_terminal()` stops processing after advance/fail
- `pipeline.agent_signal.is_some()` stops processing after explicit signal
- `increment_action_attempt` with `attempts=1` prevents double-fire from both hook and log watcher
- Stale agent_id check prevents cross-step events

### Defense-in-Depth: Log-Based Watcher Stays

The log-based file watcher (producing `AgentWaiting` events) remains active. It serves as a fallback if:
- The `oj` binary isn't on PATH in the hook's execution environment
- The daemon socket is temporarily unreachable
- Hook execution fails silently

The attempt-tracking mechanism ensures that if the hook fires first, the log-based watcher's subsequent `AgentWaiting` event is a no-op (attempts already exhausted).

### on_prompt vs on_idle

- `on_idle`: fires when agent is idle at the main prompt (waiting for user input). Default: `nudge`.
- `on_prompt`: fires when agent shows a permission/plan-approval prompt. Default: `escalate`.

Permission prompts are distinct from idle — an agent asking for permission hasn't finished working, it needs authorization to continue. Escalating by default ensures a human reviews the request.

## Verification Plan

1. **Unit tests** — `cargo test --all` covers event serialization, hook injection, precedence logic, and default configurations
2. **Clippy + fmt** — `cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings`
3. **Integration** — `quench check` for end-to-end runbook execution
4. **Manual smoke test** — Run a pipeline with an agent that triggers a permission prompt; verify instant escalation vs the old 3-minute delay
5. **Full gate** — `make check` (fmt, clippy, quench, test, build, audit, deny)
