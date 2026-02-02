# Execution Model

Two abstractions sit beneath runbooks, decoupling "what to run" from "where to run it."

```text
Runbook layer:    command → pipeline → step → agent
                                         │
Execution layer:               workspace + session
                                         │
Adapter layer:              AgentAdapter + SessionAdapter
```

## Workspace

An **isolated directory for work** -- typically populated by a pipeline's init step.

A workspace provides:
- **Identity**: Unique name for this work context
- **Isolation**: Separate from other concurrent work
- **Lifecycle**: Created before work, cleaned up after (ephemeral) or kept (persistent)
- **Context**: Values tasks can reference (`${workspace.root}`, `${workspace.id}`)

### Workspace Modes

| Mode | Behavior |
|------|----------|
| `ephemeral` | Deleted on successful pipeline completion, kept on failure for debugging |
| `persistent` | Never automatically deleted |

**Storage location**: `~/.local/state/oj/workspaces/ws-<pipeline-name>-<nonce>/`

Using XDG state directory keeps the project directory clean and survives `git clean` operations.

### Workspace Setup

The engine creates an empty directory. The pipeline's init step populates it -- typically by cloning a git repository:

```hcl
step "init" {
  run = <<-SHELL
    git clone "$(git -C ${invoke.dir} rev-parse --show-toplevel)" .
  SHELL
  on_done = { step = "work" }
}
```

This keeps the engine git-agnostic. Multi-repo workspaces are supported by cloning multiple repositories into subdirectories.

**Settings sync**: Agent-specific settings (including the Stop hook for `oj agent hook stop`) are stored in `~/.local/state/oj/agents/<agent-id>/claude-settings.json` and passed to the agent via `--settings`. Project settings from `<workspace>/.claude/settings.json` are loaded (if they exist) and merged into these agent-specific settings.

## Session

An **execution environment for an agent** -- where Claude actually runs.

Sessions are managed through two adapter layers:

| Layer | Adapter | Responsibility |
|-------|---------|----------------|
| High-level | `AgentAdapter` | Agent lifecycle, prompts, state detection |
| Low-level | `SessionAdapter` | tmux operations (spawn, send, kill) |

A session provides:
- **Isolation**: Separate process/environment
- **Monitoring**: State detection for stuck agents
- **Control**: Nudge, restart, or kill stuck sessions

### Session Properties

| Property | Description |
|----------|-------------|
| `id` | Session identifier (tmux session name, prefixed with `oj-`) |
| `cwd` | Working directory (typically the workspace path, or agent `cwd` override) |
| `env` | Environment variables passed to the agent |

### Agent State Detection

The `AgentAdapter` monitors agent state via Claude's JSONL session log:

```hcl
agent "fix" {
  on_idle  = { action = "nudge", message = "Continue working on the task." }
  on_dead  = { action = "recover", message = "Previous attempt exited. Try again." }
  on_error = "escalate"
}
```

**State detection from session log:**

| State | Log Indicator | Trigger |
|-------|--------------|---------|
| Working | `type: "assistant"` with `tool_use` or `thinking` content blocks, or `type: "user"` (processing tool results) | Keep monitoring |
| Waiting for input | `type: "assistant"` with no `tool_use` content blocks | `on_idle` (after idle timeout) |
| API error | Error field in log entry (unauthorized, quota, network, rate limit) | `on_error` |

The watcher applies an idle timeout (default 180s, configurable via `OJ_IDLE_TIMEOUT_MS`) before emitting the `WaitingForInput` state, to avoid false positives from brief pauses between tool calls.

**Process exit detection:**

| Check | Method | Trigger |
|-------|--------|---------|
| tmux alive | `tmux has-session` | `SessionGone` |
| Agent alive | `ps -p <pane_pid>` + `pgrep -P <pane_pid> -f <process>` | `Exited { exit_code }` → `on_dead` |

**Why log-based detection works**: Claude Code writes structured JSONL logs. When an assistant message has no `tool_use` content blocks, Claude has finished its current turn and is waiting for input -- the exact moment to nudge.

Agents can run indefinitely. There's no timeout.

### Why No Step Timeout?

This is a deliberate design decision, not an oversight. Step timeouts are intentionally not
supported for agent steps. Here's why:

**This is a dynamic, monitored system**

Agents and pipelines are actively monitored by both automated systems (`on_idle`, `on_dead`,
`on_error` handlers) and human operators. When something goes wrong, these monitoring systems
detect the actual problem and respond appropriately -- not by guessing that "too much time passed."

**Agents may legitimately run for extended periods**

Agents may eventually work on complex tasks that take days or weeks of actual productive work.
A timeout would arbitrarily kill legitimate work. The system needs to distinguish between
"working for a long time" and "stuck" -- which timeouts cannot do.

**Timeouts hide the real problem**

If an agent is stuck, a timeout just restarts it without understanding why. The `on_idle` and
`on_dead` monitoring detects the actual state:
- `on_idle`: Agent is waiting for input (stuck on a prompt)
- `on_dead`: Agent process exited unexpectedly
- `on_error`: Agent hit an API or system error

These tell you *what* went wrong, not just that time passed.

**The right default is NO timeout**

If a timeout feature existed, the default should be "no timeout" (infinite). But having an
infinite-default timeout is the same as not having the feature, with extra complexity and
the risk of accidental misconfiguration.

## Relationship to Runbooks

```
┌─────────────────────────────────────────────────────────────┐
│  Runbook                                                    │
│  ┌─────────────┐     ┌─────────────┐    ┌─────────────┐     │
│  │  Command    │────►│  Pipeline   │───►│    Agent    │     │
│  └─────────────┘     └─────────────┘    └─────────────┘     │
│                      ┌─────────────┐    ┌─────────────┐     │
│                      │   Worker    │───►│    Queue    │     │
│                      └─────────────┘    └─────────────┘     │
└─────────────────────────────────────────────────────────────┘
                            │                   │
                            ▼                   ▼
┌─────────────────────────────────────────────────────────────┐
│  Execution                                                  │
│  ┌─────────────┐         ┌─────────────┐                    │
│  │  Workspace  │◄────────│   Session   │                    │
│  │ (directory) │         │   (tmux)    │                    │
│  └─────────────┘         └─────────────┘                    │
└─────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────┐
│  Adapters                                                   │
│  ┌─────────────┐         ┌──────────────┐                   │
│  │ AgentAdapter│────────►│SessionAdapter│                   │
│  │  (Claude)   │         │   (tmux)     │                   │
│  └─────────────┘         └──────────────┘                   │
└─────────────────────────────────────────────────────────────┘
```

- **Pipeline** creates and owns a **Workspace**
- **Agent** runs in a **Session** within that workspace
- **AgentAdapter** manages the agent lifecycle using **SessionAdapter** for tmux operations
- Session's `cwd` points to the workspace path (or an agent-specific `cwd` override)
- Multiple agents in a pipeline share the same workspace
- **Worker** polls a **Queue** and dispatches items to pipelines

## Summary

| Concept | Purpose | Implementation |
|---------|---------|----------------|
| **Workspace** | Isolated work directory | Empty directory, populated by init step |
| **Session** | Where agent runs | Tmux session |
| **AgentAdapter** | Agent lifecycle management | ClaudeAgentAdapter |
| **SessionAdapter** | Low-level session ops | TmuxAdapter |

These abstractions enable the same runbook to work across different environments. The runbook defines *what* to do; the execution layer handles *where* and *how*.
