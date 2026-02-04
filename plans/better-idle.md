# Better Idle Detection via Notification Hooks

## Overview

Replace the current log inactivity-based idle detection with Claude Code's built-in Notification hook system. Instead of polling the JSONL session log and applying timeout heuristics, install a `Notification` hook that fires immediately when Claude Code signals `idle_prompt` (agent idle) or `permission_prompt` (needs escalation). This provides faster, more reliable idle detection without arbitrary timeouts.

## Project Structure

```
crates/
├── cli/src/commands/agent.rs   # Add `oj agent hook notify` command
├── adapters/src/agent/
│   ├── claude.rs               # Update hook installation during spawn
│   ├── watcher.rs              # Simplify: remove log-based idle detection
│   └── hooks.rs                # NEW: Hook configuration generation
├── core/src/event.rs           # Update Event::AgentIdle payload (if needed)
└── engine/src/runtime/
    └── handlers/agent.rs       # Verify idle/escalate event handling
```

## Dependencies

No new external dependencies. Uses existing:
- `serde_json` for JSON parsing (stdin from Claude Code)
- Tokio channels for daemon communication
- Existing daemon socket infrastructure

## Implementation Phases

### Phase 1: Add `oj agent hook notify` Command

**Goal**: Implement the CLI command that Claude Code invokes as a Notification hook.

**Files to modify**:
- `crates/cli/src/commands/agent.rs`

**Implementation**:

Add a new `Notify` variant to the `Hook` subcommand enum (alongside existing `Stop` and `Pretooluse`):

```rust
#[derive(Parser, Debug)]
pub enum Hook {
    /// Stop hook - gates agent completion
    Stop {
        #[arg(long)]
        agent_id: String,
    },
    /// PreToolUse hook - detects plan/question tools
    Pretooluse {
        #[arg(long)]
        agent_id: String,
    },
    /// Notification hook - detects idle_prompt and permission_prompt
    Notify {
        #[arg(long)]
        agent_id: String,
    },
}
```

Implement `handle_notify_hook()`:

```rust
#[derive(Debug, Deserialize)]
struct NotificationHookInput {
    notification_type: String,
    message: Option<String>,
    // Common fields from Claude Code
    session_id: String,
    hook_event_name: String,
}

async fn handle_notify_hook(agent_id: &str, client: &DaemonClient) -> Result<()> {
    let mut input_json = String::new();
    io::stdin().read_to_string(&mut input_json)?;

    let input: NotificationHookInput = serde_json::from_str(&input_json)
        .context("Failed to parse notification hook input")?;

    match input.notification_type.as_str() {
        "idle_prompt" => {
            // Agent is waiting for user input - signal idle state
            client.emit_agent_idle(agent_id).await?;
        }
        "permission_prompt" => {
            // Agent needs permission - signal escalation needed
            client.emit_agent_escalate(agent_id, "permission_required").await?;
        }
        _ => {
            // Ignore other notification types (auth_success, elicitation_dialog, etc.)
        }
    }

    // Always exit 0 - notification hooks cannot block
    std::process::exit(0);
}
```

**Verification**:
- Unit test parsing different `notification_type` values
- Integration test that spawns an agent and verifies hook receives notifications

---

### Phase 2: Generate Hooks Configuration

**Goal**: Create a module to generate the Claude Code hooks.json configuration that gets installed during agent spawn.

**Files to create**:
- `crates/adapters/src/agent/hooks.rs`

**Implementation**:

```rust
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct HooksConfig {
    pub hooks: HashMap<String, Vec<MatcherGroup>>,
}

#[derive(Debug, Serialize)]
pub struct MatcherGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub hooks: Vec<HookHandler>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum HookHandler {
    #[serde(rename = "command")]
    Command {
        command: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout: Option<u32>,
    },
}

impl HooksConfig {
    pub fn for_agent(agent_id: &str, state_dir: &Path) -> Self {
        let oj_bin = std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("oj"));

        let mut hooks = HashMap::new();

        // Notification hook for idle_prompt and permission_prompt
        hooks.insert("Notification".to_string(), vec![
            MatcherGroup {
                matcher: Some("idle_prompt|permission_prompt".to_string()),
                hooks: vec![HookHandler::Command {
                    command: format!(
                        "OJ_STATE_DIR={} {} agent hook notify --agent-id {}",
                        state_dir.display(),
                        oj_bin.display(),
                        agent_id
                    ),
                    timeout: Some(30),
                }],
            },
        ]);

        // Stop hook (existing)
        hooks.insert("Stop".to_string(), vec![
            MatcherGroup {
                matcher: None,
                hooks: vec![HookHandler::Command {
                    command: format!(
                        "OJ_STATE_DIR={} {} agent hook stop --agent-id {}",
                        state_dir.display(),
                        oj_bin.display(),
                        agent_id
                    ),
                    timeout: Some(30),
                }],
            },
        ]);

        // PreToolUse hook for plan/question tools (existing)
        hooks.insert("PreToolUse".to_string(), vec![
            MatcherGroup {
                matcher: Some("AskUserQuestion|EnterPlanMode".to_string()),
                hooks: vec![HookHandler::Command {
                    command: format!(
                        "OJ_STATE_DIR={} {} agent hook pretooluse --agent-id {}",
                        state_dir.display(),
                        oj_bin.display(),
                        agent_id
                    ),
                    timeout: Some(30),
                }],
            },
        ]);

        HooksConfig { hooks }
    }

    pub fn write_to(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
    }
}
```

**Verification**:
- Unit test that generated JSON matches expected Claude Code schema
- Test round-trip: generate, write, read back

---

### Phase 3: Install Hooks During Agent Spawn

**Goal**: Write the hooks.json file to the agent's workspace `.claude/` directory during spawn, so Claude Code automatically loads it.

**Files to modify**:
- `crates/adapters/src/agent/claude.rs`
- `crates/adapters/src/agent/mod.rs` (re-export hooks module)

**Implementation**:

In `ClaudeAgentAdapter::spawn()`, after creating the workspace directory:

```rust
// Create .claude directory in workspace
let claude_dir = workspace_path.join(".claude");
std::fs::create_dir_all(&claude_dir)?;

// Generate and write hooks configuration
let hooks_config = HooksConfig::for_agent(&agent_id, &state_dir);
let settings_path = claude_dir.join("settings.json");
hooks_config.write_to(&settings_path)?;

tracing::debug!(
    agent_id = %agent_id,
    settings_path = %settings_path.display(),
    "Installed Claude Code hooks"
);
```

**Key consideration**: The hooks.json must be in place BEFORE the Claude process starts, so Claude Code loads it on startup. The current spawn sequence already creates the workspace before launching tmux, so this should work naturally.

**Verification**:
- Integration test that spawns agent, then checks `.claude/settings.json` exists with correct content
- Verify Claude Code actually loads the hooks (check hook fires on notification)

---

### Phase 4: Simplify Watcher - Remove Log-Based Idle Detection

**Goal**: Remove the log inactivity timeout logic from the watcher. The watcher should still monitor for process death and session-gone states, but idle detection now comes from the Notification hook.

**Files to modify**:
- `crates/adapters/src/agent/watcher.rs`

**Changes**:

1. **Remove idle timeout logic** (lines 231-238, 286-290):
   - Remove `idle_timeout` configuration and `OJ_IDLE_TIMEOUT_MS` env var
   - Remove `idle_emitted` tracking
   - Remove the idle timeout check in the watch loop

2. **Keep essential monitoring**:
   - File watcher for log changes (still useful for detecting state transitions)
   - Process liveness checks (detect unexpected exits)
   - Session existence checks (detect session-gone)
   - Error detection from log parsing (detect failures)

3. **Simplify state emission**:
   - The watcher should still emit `Working` state when it detects activity
   - The watcher should emit `Failed` when it detects errors in the log
   - The watcher should emit `Exited` when the process dies
   - The watcher should NOT emit `WaitingForInput` based on timeout - this now comes from the hook

**Before** (simplified):
```rust
// Old: timeout-based idle detection
if !idle_emitted && clock.now().duration_since(last_activity) >= idle_timeout
    && last_state == AgentState::WaitingForInput {
    idle_emitted = true;
    event_tx.send(Event::from_agent_state(agent_id, AgentState::WaitingForInput));
}
```

**After**:
```rust
// New: idle detection comes from Notification hook
// Watcher only detects Working, Failed, Exited, SessionGone
// WaitingForInput events come from the notify hook via daemon
```

**Verification**:
- Existing tests should still pass (adjust expectations for idle events)
- Verify watcher still detects process death correctly
- Verify watcher still detects errors in log

---

### Phase 5: Update Event Handling for Hook-Based Idle

**Goal**: Ensure the daemon and engine properly handle idle events coming from the Notification hook instead of the watcher.

**Files to review/modify**:
- `crates/daemon/src/service.rs` - Add `emit_agent_idle` and `emit_agent_escalate` handlers
- `crates/core/src/event.rs` - Verify `agent:idle` event type exists
- `crates/engine/src/runtime/handlers/agent.rs` - Verify idle handling works

**Implementation**:

Add daemon RPC methods if not present:

```rust
// In daemon service
async fn emit_agent_idle(&self, agent_id: &str) -> Result<()> {
    let event = Event::AgentIdle {
        agent_id: AgentId::new(agent_id),
        timestamp: Utc::now(),
    };
    self.event_tx.send(event).await?;
    Ok(())
}

async fn emit_agent_escalate(&self, agent_id: &str, reason: &str) -> Result<()> {
    let event = Event::AgentEscalate {
        agent_id: AgentId::new(agent_id),
        reason: reason.to_string(),
        timestamp: Utc::now(),
    };
    self.event_tx.send(event).await?;
    Ok(())
}
```

Verify engine handler routes `AgentIdle` to trigger `on_idle` actions (this should already exist).

**Verification**:
- Integration test: spawn agent, trigger idle_prompt notification, verify on_idle action fires
- Integration test: spawn agent, trigger permission_prompt notification, verify escalation occurs

---

### Phase 6: End-to-End Testing and Cleanup

**Goal**: Comprehensive testing and removal of deprecated code paths.

**Files to modify**:
- `tests/specs/agent/hooks.rs` - Add notification hook tests
- `tests/specs/agent/idle.rs` - Update idle detection tests
- Various files - Remove dead code

**Test scenarios**:

1. **Idle detection via hook**:
   - Spawn agent with Claude
   - Simulate Claude sending `idle_prompt` notification
   - Verify `on_idle` action triggers within seconds (not 180s)

2. **Escalation via hook**:
   - Spawn agent with Claude
   - Simulate Claude sending `permission_prompt` notification
   - Verify agent transitions to escalated state

3. **Process death detection** (unchanged):
   - Kill Claude process
   - Verify `on_dead` action still triggers

4. **Hook installation**:
   - Spawn agent
   - Verify `.claude/settings.json` exists in workspace
   - Verify JSON contains Notification hook with correct matcher

**Cleanup**:
- Remove `OJ_IDLE_TIMEOUT_MS` environment variable handling
- Remove idle timeout documentation
- Update CLAUDE.md if it mentions idle detection internals

## Key Implementation Details

### Hook JSON Schema

Claude Code expects hooks in this format (project `.claude/settings.json`):

```json
{
  "hooks": {
    "Notification": [
      {
        "matcher": "idle_prompt|permission_prompt",
        "hooks": [
          {
            "type": "command",
            "command": "oj agent hook notify --agent-id <id>",
            "timeout": 30
          }
        ]
      }
    ],
    "Stop": [...],
    "PreToolUse": [...]
  }
}
```

### Notification Hook Input

Claude Code sends this JSON to stdin:

```json
{
  "session_id": "abc123",
  "transcript_path": "/path/to/transcript.jsonl",
  "cwd": "/path/to/workspace",
  "permission_mode": "default",
  "hook_event_name": "Notification",
  "message": "Claude Code needs your attention",
  "title": "Idle",
  "notification_type": "idle_prompt"
}
```

### State Mapping

| notification_type | Action | Effect |
|-------------------|--------|--------|
| `idle_prompt` | Emit `AgentIdle` event | Triggers `on_idle` action |
| `permission_prompt` | Emit `AgentEscalate` event | Triggers escalation flow |
| Other | Ignore | No action |

### Backward Compatibility

The watcher remains for:
- Error detection from log parsing
- Process death detection
- Session existence monitoring

Only idle detection moves to hooks. This means existing `on_dead` configurations continue working unchanged.

## Verification Plan

### Unit Tests

1. **Hook input parsing**: Test `NotificationHookInput` deserialization with various `notification_type` values
2. **Hooks config generation**: Test `HooksConfig::for_agent()` produces valid JSON
3. **Matcher patterns**: Verify regex `"idle_prompt|permission_prompt"` matches correctly

### Integration Tests

1. **Hook installation**: Spawn agent, verify `.claude/settings.json` created correctly
2. **Idle detection**: Mock notification, verify event emitted to daemon
3. **Escalation**: Mock permission_prompt, verify escalation event
4. **Full pipeline**: Run agent through idle → nudge → complete cycle

### Manual Testing

1. Run `oj run build --name=test-feature` with a real Claude session
2. Let Claude finish and go idle
3. Verify idle detection happens quickly (not 180s timeout)
4. Verify `on_idle` action (nudge/done/escalate) triggers correctly

### Performance

- Hook invocation should be <100ms (simple CLI call)
- No polling overhead for idle detection
- File watcher remains for error/exit detection (low overhead)
