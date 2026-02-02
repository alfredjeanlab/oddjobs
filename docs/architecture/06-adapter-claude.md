# Claude Code Integration

How Claude Code runs within the oj orchestration system via the `AgentAdapter` abstraction.

## AgentAdapter

Claude Code is managed through the `AgentAdapter` trait, with `ClaudeAgentAdapter` as the production implementation. This abstraction encapsulates all Claude-specific behavior:

```rust
// Engine uses AgentAdapter, not raw sessions
let handle = agents.spawn(config, event_tx).await?;
let state = agents.get_state(&agent_id).await?;
agents.send(&agent_id, "Continue working...").await?;
agents.kill(&agent_id).await?;
agents.reconnect(reconnect_config, event_tx).await?;
```

The `ClaudeAgentAdapter`:
- Uses `SessionAdapter` internally for tmux operations
- Auto-accepts bypass permissions, workspace trust, and login prompts after spawn
- Watches Claude's JSONL session log via file notifications for state detection
- Handles Claude-specific error patterns

See [Adapters](05-adapters.md) for the full trait definition.

## Prompt Handling

Claude Code shows interactive prompts that require acknowledgement:

| Prompt | Trigger | Auto-Response |
|--------|---------|---------------|
| Bypass permissions prompt | `--dangerously-skip-permissions` in command | Sends "2" to accept |
| Workspace trust prompt (spawn) | "Accessing workspace" + "1. Yes" / "2. No" text detected | Sends "1" to trust folder |
| Workspace trust prompt (watcher) | "Do you trust the files in this folder?" text detected during log wait | Sends "y" to trust folder |
| Login/onboarding prompt | Unauthenticated Claude Code detected | Kills session, returns `AgentError` |

The `ClaudeAgentAdapter` detects spawn-time prompts via `capture_output()` polling (configurable via `OJ_PROMPT_POLL_MS`, default 3000ms = 15 attempts at 200ms intervals) and automatically sends the appropriate response, allowing agents to start without manual intervention. The watcher handles trust prompts that appear later during session log initialization.

## Sessions

Claude Code runs in tmux sessions (via `SessionAdapter`) for:
- **Isolation**: Separate environment per agent
- **Output capture**: Monitor terminal for prompt detection
- **Input injection**: Send messages to nudge stuck agents
- **Clean termination**: Kill sessions when stuck or complete

Session names follow the format `oj-{pipeline}-{agent_name}-{random}`, where the `oj-` prefix is added by `TmuxAdapter`, pipeline and agent names are sanitized and truncated (20 and 15 characters respectively), and a 4-character random suffix ensures uniqueness. The agent UUID is used as `--session-id` for Claude's log file, while the friendly tmux session name is used for all tmux operations.

The orchestrator creates, monitors, and destroys sessions. Claude Code doesn't manage its own lifecycle.

## State Detection

Agent state is detected from Claude's JSONL session log by a background watcher (file notifications + periodic polling, configurable via `OJ_WATCHER_POLL_MS`, default 5 seconds):

| State | Log Indicator | Trigger |
|-------|--------------|---------|
| Working | `type: "assistant"` with `tool_use` or `thinking` content blocks, or `type: "user"` (processing tool results) | Keep monitoring |
| Waiting for input | `type: "assistant"` with `stop_reason: null` and no `tool_use` content blocks | `on_idle` (after idle timeout) |
| API error | Error fields in log entry (unauthorized, quota, network, rate limit) | `on_error` |

The watcher applies an idle timeout (default 180s, configurable via `OJ_IDLE_TIMEOUT_MS`) before emitting `AgentWaiting`, to avoid false positives from brief pauses between tool calls.

**Process exit detection:**

| Check | Method | Trigger |
|-------|--------|---------|
| tmux alive | `tmux has-session` | `SessionGone` |
| Agent alive | `ps -p <pane_pid> -o command=` (check pane process) + `pgrep -P <pane_pid> -f <process>` (check children) | `Exited { exit_code }` → `on_dead` |

**Why log-based detection works**: Claude Code writes structured JSONL logs. When an assistant message has no `tool_use` content blocks, Claude has finished its current turn and is waiting for input - the exact moment to nudge.

## Stuck Recovery

1. **Detect**: Background watcher emits `AgentWaiting` event (after idle timeout)
2. **Nudge**: Engine sends follow-up message via `Effect::SendToSession`
3. **Escalate**: If nudge doesn't help, desktop notification via `Effect::Notify`

Nudging works because Claude Code accepts user input via the terminal. A nudge message acts like typing a follow-up prompt, encouraging the agent to resume work.

**Recovery commands for humans:**
```bash
oj session attach <id>         # Attach to see what's happening
oj session send <id> "message" # Send follow-up message
oj pipeline resume <id>        # Resume monitoring
```

**Failure detection:**
The session log also reveals errors that require escalation:
- **Unauthorized** (`AgentError::Unauthorized`): Invalid API key
- **Out of credits** (`AgentError::OutOfCredits`): Billing/quota exceeded
- **Network error** (`AgentError::NoInternet`): Connection failed
- **Rate limited** (`AgentError::RateLimited`): Too many requests

These are categorized as `Failed` states and trigger `on_error` actions.

## Shell Commands

Expose orchestration via allowed shell commands in agent settings:

```json
{ "allowed": ["Bash(oj:*)"] }
```

Then agents signal completion:
```bash
# Signal successful completion (advances pipeline to next step)
oj emit agent:signal --agent <id> '{"kind": "complete"}'

# Signal escalation (pauses pipeline, notifies human)
oj emit agent:signal --agent <id> '{"kind": "escalate", "message": "Need human review"}'
```

The JSON payload accepts:
- `kind` (or alias `action`): `"complete"` or `"escalate"` (required)
- `message`: optional explanation string

**How agents learn their ID:** The orchestrator injects a Stop hook into each agent's settings at `$OJ_STATE_DIR/agents/<agent-id>/claude-settings.json`, passed via `--settings`. The hook command is `oj agent hook stop <agent_id>`. When the agent tries to exit without signaling, the hook blocks and returns a message with the exact `oj emit agent:signal --agent <id> ...` command to run.

When agents fail to signal completion, the `on_idle` and `on_dead` actions act as a safety net to advance agentic workflows.

## Hooks

Claude Code hooks intercept execution. Only the **Stop** hook is automatically configured by the orchestrator:

**Stop**: Gates agent exit until it signals completion via `oj emit agent:signal`.

Generated settings format:
```json
{
  "hooks": {
    "Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "oj agent hook stop <agent_id>"}]}]
  }
}
```

## Testing

Use [claudeless](https://github.com/anthropics/claudeless) for integration testing. It's a CLI simulator that emulates Claude's interface, TUI, hooks, and permissions without API costs.

```bash
# Run tests with claudeless instead of real claude
PATH="$CLAUDELESS_BIN:$PATH" cargo test
```

Scenario files control responses, making tests deterministic. The `ClaudeAgentAdapter` works identically with both real Claude and claudeless.

For unit tests, use `FakeAgentAdapter` to test engine logic without any subprocess execution.

## Summary

| Integration | Direction | Purpose |
|-------------|-----------|---------|
| **AgentAdapter** | Engine → Claude | Lifecycle, prompts, state detection |
| **SessionAdapter** | AgentAdapter → tmux | Low-level session operations |
| **Shell commands** | Claude → Engine | Signaling, events |
| **Stop hook** | Claude → Engine | Gate exit until agent signals completion |
| **CLAUDE.md** | Static → Claude | Agent instructions |
| **Env vars** | External → Claude | Runtime context (`OJ_STATE_DIR`, `CLAUDE_CONFIG_DIR`) |
| **Claudeless** | Testing | Deterministic integration tests |
| **FakeAgentAdapter** | Testing | Unit tests without subprocesses |
