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
| Login/onboarding prompt | Unauthenticated Claude Code detected | Kills session, returns `AgentAdapterError` |

The `ClaudeAgentAdapter` detects spawn-time prompts via `capture_output()` polling (configurable via `OJ_PROMPT_POLL_MS`, default 3000ms = 15 attempts at 200ms intervals) and automatically sends the appropriate response, allowing agents to start without manual intervention. The watcher handles trust prompts that appear later during session log initialization.

## Sessions

Claude Code runs in tmux sessions (via `SessionAdapter`) for:
- **Isolation**: Separate environment per agent
- **Output capture**: Monitor terminal for prompt detection
- **Input injection**: Send messages to nudge stuck agents
- **Clean termination**: Kill sessions when stuck or complete

Session names follow the format `oj-{job}-{agent_name}-{random}`, where the `oj-` prefix is added by `TmuxAdapter`, job and agent names are sanitized and truncated (20 and 15 characters respectively), and a 4-character random suffix ensures uniqueness. The agent UUID is used as `--session-id` for Claude's log file, while the friendly tmux session name is used for all tmux operations.

The orchestrator creates, monitors, and destroys sessions. Claude Code doesn't manage its own lifecycle.

## State Detection

Agent state is detected via two mechanisms:

1. **Notification hook** (primary): A `Notification` hook installed in Claude's settings fires immediately when Claude signals `idle_prompt` or `permission_prompt`. The hook calls `oj agent hook notify --agent-id <id>`, which emits `AgentIdle` or `AgentPrompt` events to the daemon.

2. **Session log watcher** (fallback): A background watcher monitors Claude's JSONL session log via file notifications + periodic polling (configurable via `OJ_WATCHER_POLL_MS`, default 5 seconds):

| State | Log Indicator | Trigger |
|-------|--------------|---------|
| Working | `type: "assistant"` with `tool_use` or `thinking` content blocks, or `type: "user"` (processing tool results) | Keep monitoring |
| Waiting for input | `type: "assistant"` with `stop_reason: null` and no `tool_use` content blocks | `on_idle` (emitted as `AgentIdle`) |
| API error | Error fields in log entry (unauthorized, quota, network, rate limit) | `on_error` |

Both mechanisms emit the same `AgentIdle` event, so the engine handles them identically. The Notification hook provides the fastest detection; the log watcher serves as a fallback.

**Process exit detection:**

| Check | Method | Trigger |
|-------|--------|---------|
| tmux alive | `tmux has-session` | `SessionGone` |
| Agent alive | `ps -p <pane_pid> -o command=` (check pane process) + `pgrep -P <pane_pid> -f <process>` (check children) | `Exited { exit_code }` → `on_dead` |

**Why log-based detection works**: Claude Code writes structured JSONL logs. When an assistant message has no `tool_use` content blocks, Claude has finished its current turn and is waiting for input - the exact moment to nudge.

## Idle Grace Timer

When an `AgentIdle` event arrives, the engine does **not** act immediately. Instead, it sets a 60-second grace timer and records the current session log file size. This prevents false idle triggers from brief text-only states between tool calls.

When the grace timer fires, two conditions must hold before proceeding with `on_idle`:

1. **Log file hasn't grown** — any activity (tool calls, thinking, subagent output, streaming) writes to the log
2. **Agent state is still `WaitingForInput`** — guards against race conditions where a tool started after the `AgentIdle` event was queued

If either check fails, the idle is cancelled as a false positive.

```diagram
Idle Grace Timer Flow:

  AgentIdle received
      │
      ├── Grace timer already pending? → Drop (deduplicate)
      │
      └── Record session log file size, set 60s grace timer
              │
              │  ... 60 seconds ...
              │
          Grace timer fires
              │
              ├── Log file grew? → Not idle (cancel)
              │
              ├── Agent state == Working? → Not idle (cancel)
              │
              └── Both checks pass → Proceed with on_idle action
```

**Working cancels the grace timer**: When the agent transitions to Working (tool_use or thinking detected), any pending idle grace timer is cancelled immediately and the recorded log size is cleared.

**Activity type coverage:**

| Activity | Why idle won't false-trigger |
|----------|------------------------------|
| Tool calls (Read, Write, Bash, etc.) | tool_use block → watcher reports Working → no AgentIdle event |
| Thinking (extended thinking) | thinking block → watcher reports Working → no AgentIdle |
| Subagents (Task/Explore) | tool_use for Task → watcher reports Working throughout subagent execution |
| Long Bash (>60s) | tool_use block persists until result → watcher reports Working |
| Background Bash (run_in_background) | Result returns immediately with task_id; agent continues normally |
| Brief text between tool calls | AgentIdle fires → grace timer set → agent calls next tool within seconds → log grows → timer cancelled |
| Streaming text response | JSONL entry written when response completes; if still generating, previous tool_use/result is last line → Working |
| Race: tool started after AgentIdle queued | Grace timer re-checks `get_agent_state()` → sees Working → no-op |

**Cross-agent isolation**: Each agent has its own session log. Agent A dispatching work via `oj run` creates a separate job/agent. Agent A's idle detection is independent — its session log reflects its own activity, not child agents'.

**Timelines:**

```
Normal work (tool calls every 3-10s):
─────────────────────────────────────────────────────────────
  t=0    text msg  → AgentIdle → set grace timer, record log size
  t=2    tool_use  → Working   → cancel grace timer ✓
  t=5    result    → log grows
  t=6    text msg  → AgentIdle → set grace timer, record log size
  t=8    tool_use  → Working   → cancel grace timer ✓
  ...    (timer never fires)

Long tool call (60+ second Bash/subagent):
─────────────────────────────────────────────────────────────
  t=0    tool_use  → Working (tool_use in last line)
  ...    (watcher sees Working — no AgentIdle fires)
  t=90   result    → Working
  t=91   text msg  → AgentIdle → set grace timer
  t=93   tool_use  → Working   → cancel grace timer ✓

Genuinely stuck:
─────────────────────────────────────────────────────────────
  t=0    text msg  → AgentIdle → set grace timer, record log size
  ...    (no log entries, no tool calls)
  t=60   timer fires → log unchanged + state WaitingForInput
         → on_idle → nudge (attempt 1)
  t=61   user msg  → Working (our nudge text)
         → suppress auto-resume (last_nudge_at < 60s)
  t=62   text msg  → AgentIdle → set grace timer
  ...    (still no real work)
  t=122  timer fires → on_idle → attempts exhausted → escalate ✓
```

**Environment variable**: `OJ_IDLE_GRACE_MS` overrides the default 60000ms grace period (used in integration tests).

## Stuck Recovery

1. **Detect**: `AgentIdle` event fires → 60s grace timer → dual check confirms genuinely idle
2. **Nudge**: Engine sends follow-up message via `Effect::SendToSession`
3. **Self-trigger prevention**: After sending a nudge, auto-resume is suppressed for 60s so our own nudge text doesn't reset the cycle
4. **Escalate**: If nudge doesn't help, desktop notification via `Effect::Notify`

Nudging works because Claude Code accepts user input via the terminal. A nudge message acts like typing a follow-up prompt, encouraging the agent to resume work.

**Recovery commands for humans:**
```bash
oj session attach <id>         # Attach to see what's happening
oj session send <id> "message" # Send follow-up message
oj job resume <id>        # Resume monitoring
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
# Signal successful completion (advances job to next step)
oj emit agent:signal --agent <id> '{"kind": "complete"}'

# Signal escalation (pauses job, notifies human)
oj emit agent:signal --agent <id> '{"kind": "escalate", "message": "Need human review"}'

# Signal continue (no-op acknowledgement — agent is still working)
oj emit agent:signal --agent <id> continue
```

The JSON payload accepts:
- `kind` (or alias `action`): `"complete"`, `"escalate"`, or `"continue"` (required)
- `message`: optional explanation string

**How agents learn their ID:** The orchestrator injects a Stop hook into each agent's settings at `$OJ_STATE_DIR/agents/<agent-id>/claude-settings.json`, passed via `--settings`. The hook command is `oj agent hook stop <agent_id>`. When the agent tries to exit without signaling, the hook blocks and returns a message with the exact `oj emit agent:signal --agent <id> ...` command to run.

When agents fail to signal completion, the `on_idle` and `on_dead` actions act as a safety net to advance agentic workflows.

## Hooks

Claude Code hooks intercept execution. The orchestrator automatically configures these hooks in the agent's settings file (`$OJ_STATE_DIR/agents/<agent-id>/claude-settings.json`):

| Hook | Matcher | Command | Purpose |
|------|---------|---------|---------|
| **Stop** | `""` (all) | `oj agent hook stop <id>` | Gates exit until agent signals completion |
| **Notification** | `idle_prompt\|permission_prompt` | `oj agent hook notify --agent-id <id>` | Instant idle/permission detection |
| **PreToolUse** | `ExitPlanMode\|AskUserQuestion\|EnterPlanMode` | `oj agent hook pretooluse <id>` | Detects plan/question tools |
| **SessionStart** | per-source | `bash <script>` | Runs prime scripts on session start |

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
| **Notification hook** | Claude → Engine | Instant idle/permission detection |
| **CLAUDE.md** | Static → Claude | Agent instructions |
| **Env vars** | External → Claude | Runtime context (`OJ_STATE_DIR`, `CLAUDE_CONFIG_DIR`) |
| **Claudeless** | Testing | Deterministic integration tests |
| **FakeAgentAdapter** | Testing | Unit tests without subprocesses |
