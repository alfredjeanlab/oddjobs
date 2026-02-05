# Execution Model

Two abstractions sit beneath runbooks, decoupling "what to run" from "where to run it."

```text
Runbook layer:    command → job → step → agent
                                         │
Execution layer:               workspace + session
                                         │
Adapter layer:              AgentAdapter + SessionAdapter
```

## Workspace

An **isolated directory for work** -- typically populated by a job's init step.

A workspace provides:
- **Identity**: Unique name for this work context
- **Isolation**: Separate from other concurrent work
- **Lifecycle**: Created before work, cleaned up on success or cancellation, kept on failure for debugging
- **Context**: Values tasks can reference (`${workspace.root}`, `${workspace.id}`, `${workspace.branch}`)

### Workspace Types

| Type | Syntax | Behavior |
|------|--------|----------|
| `folder` | `workspace = "folder"` | Plain directory. Engine creates the directory; the init step populates it. |
| `worktree` | `workspace { git = "worktree" }` | Engine-managed git worktree. The engine handles `git worktree add`, `git worktree remove`, and branch cleanup automatically. |

**Storage location**: `~/.local/state/oj/workspaces/ws-<job-name>-<nonce>/`

Using XDG state directory keeps the project directory clean and survives `git clean` operations.

### Workspace Setup

For `workspace { git = "worktree" }`, the engine creates a git worktree automatically. The branch name comes from `workspace.branch` if set, otherwise `ws-<nonce>`. The start point comes from `workspace.ref` if set, otherwise `HEAD`. Both fields support `${var.*}` and `${workspace.*}` interpolation; `ref` also supports `$(...)` shell expressions. The `${workspace.branch}` template variable is available in step templates.

```hcl
workspace {
  git    = "worktree"
  branch = "feature/${var.name}-${workspace.nonce}"
  ref    = "origin/main"
}
```

For `workspace = "folder"`, the engine creates an empty directory. The job's init step populates it -- useful when fully custom worktree management is needed:

```hcl
step "init" {
  run = <<-SHELL
    git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" origin/${var.base}
  SHELL
  on_done = { step = "work" }
}
```

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
| Waiting for input | `type: "assistant"` with no `tool_use` content blocks | `on_idle` (after 60s grace period) |
| API error | Error field in log entry (unauthorized, quota, network, rate limit) | `on_error` |

When idle is detected, the engine applies a 60-second grace period before firing `on_idle`. During this grace period, the engine checks for any session log activity and re-verifies the agent state, preventing false idle triggers from brief pauses between tool calls. See [Claude Code adapter](../arch/06-adapter-claude.md) for detailed idle detection mechanics.

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

Agents and jobs are actively monitored by both automated systems (`on_idle`, `on_dead`,
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
│  │  Command    │────►│   Job       │───►│    Agent    │     │
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

- **Job** creates and owns a **Workspace**
- **Agent** runs in a **Session** within that workspace
- **AgentAdapter** manages the agent lifecycle using **SessionAdapter** for tmux operations
- Session's `cwd` points to the workspace path (or an agent-specific `cwd` override)
- Multiple agents in a job share the same workspace
- **Worker** polls a **Queue** and dispatches items to jobs

## Summary

| Concept | Purpose | Implementation |
|---------|---------|----------------|
| **Workspace** | Isolated work directory | Empty directory, populated by init step |
| **Session** | Where agent runs | Tmux session |
| **AgentAdapter** | Agent lifecycle management | ClaudeAgentAdapter |
| **SessionAdapter** | Low-level session ops | TmuxAdapter |

These abstractions enable the same runbook to work across different environments. The runbook defines *what* to do; the execution layer handles *where* and *how*.
