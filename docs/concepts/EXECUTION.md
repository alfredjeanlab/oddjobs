# Execution Model

Two abstractions sit beneath runbooks, decoupling "what to run" from "where to run it."

```text
Runbook layer:    command → job → step → agent
                                         │
Execution layer:               workspace + session
                                         │
Adapter layer:              AgentAdapter
```

## Workspace

An **isolated directory for work** -- typically populated by a job's init step.

A workspace provides:
- **Identity**: Unique name for this work context
- **Isolation**: Separate from other concurrent work
- **Lifecycle**: Created before work, cleaned up on success or cancellation, kept on failure for debugging
- **Context**: Values tasks can reference (`${source.root}`, `${source.id}`, `${source.branch}`)

### Workspace Types

| Type | Syntax | Behavior |
|------|--------|----------|
| `folder` | `source = "folder"` | Plain directory. Engine creates the directory; the init step populates it. |
| `worktree` | `source { git = true }` | Engine-managed git worktree. The engine handles `git worktree add`, `git worktree remove`, and branch cleanup automatically. |

**Local execution**: Workspaces are stored at
`~/.local/state/oj/workspaces/ws-<job-name>-<nonce>/`. Using XDG state directory
keeps the project directory clean and survives `git clean` operations.

**Containerized execution**: Code is provisioned via `git clone` into an
`emptyDir` volume (K8s) or Docker volume. An init container clones the repo
before the main agent container starts. The daemon resolves the repo URL and
branch from the job's `source` block at creation time — no local checkout is
needed. See [Containers](../arch/07-containers.md) for details.

### Workspace Setup

For `source { git = true }`, the engine creates a git worktree automatically. The branch name comes from `source.branch` if set, otherwise `ws-<nonce>`. The start point comes from `source.ref` if set, otherwise `HEAD`. Both fields support `${var.*}` and `${source.*}` interpolation; `ref` also supports `$(...)` shell expressions. The `${source.branch}` template variable is available in step templates.

```hcl
source {
  git    = true
  branch = "feature/${var.name}-${source.nonce}"
  ref    = "origin/main"
}
```

For `source = "folder"`, the engine creates an empty directory. The job's init step populates it -- useful when fully custom worktree management is needed:

```hcl
step "init" {
  run = <<-SHELL
    git -C "${local.repo}" worktree add -b "${local.branch}" "${source.root}" origin/${var.base}
  SHELL
  on_done = { step = "work" }
}
```

**Agent config**: Agent configuration (including settings, stop gate config, and SessionStart hooks for prime scripts) is stored in `~/.local/state/oj/agents/<agent-id>/agent-config.json` and passed to the agent via `--agent-config`. Project settings from `<workspace>/.claude/settings.json` are loaded (if they exist) and merged into the agent-specific settings.

## Session

An **execution environment for an agent** -- where Claude actually runs.

Sessions are managed through two adapter layers:

| Layer | Adapter | Responsibility |
|-------|---------|----------------|
| High-level | `AgentAdapter` | Agent lifecycle, prompts, state detection, process management |

The `RuntimeRouter` selects the adapter based on environment and agent config:

| Environment | Adapter | Transport |
|-------------|---------|-----------|
| Local (no container) | `LocalAdapter` | Unix socket |
| Local (with container) | `DockerAdapter` | TCP |
| Kubernetes | `KubernetesAdapter` | TCP |

A session provides:
- **Isolation**: Separate process/environment
- **Monitoring**: State detection for stuck agents
- **Control**: Nudge, restart, or kill stuck sessions

### Session Properties

| Property | Description |
|----------|-------------|
| `id` | Session identifier (agent session name, prefixed with `oj-`) |
| `cwd` | Working directory (typically the workspace path, or agent `cwd` override) |
| `env` | Environment variables passed to the agent |

### Agent State Detection

The `AgentAdapter` monitors agent state via coop's WebSocket event bridge:

```hcl
agent "fix" {
  on_idle  = { action = "nudge", message = "Continue working on the task." }
  on_dead  = { action = "recover", message = "Previous attempt exited. Try again." }
  on_error = "escalate"
}
```

**State detection via coop WebSocket:**

| Coop State | Engine Event | Trigger |
|------------|-------------|---------|
| `working` | `AgentWorking` | Agent is processing (tool use or thinking) |
| `idle` | `AgentIdle` | Agent finished current turn, waiting for input |
| `prompt` | `AgentPrompt` | Permission, plan approval, or question prompt |
| `error` | `AgentFailed` | API error (unauthorized, quota, network, rate limit) |
| `exited` / WS close | `AgentGone` | Agent process exited |
| `stop:outcome` (blocked) | `AgentStopBlocked` | Agent tried to exit but stop gate blocked it |

When idle is detected, the engine applies a 60-second grace period before firing `on_idle`. During this grace period, the engine re-verifies the agent state, preventing false idle triggers from brief pauses between tool calls. See [Agents](../arch/05-agents.md) for detailed idle detection mechanics.

**Process exit detection:** Coop monitors the agent process directly and emits exit events with the exit code via the WebSocket connection, triggering `Exited { exit_code }` and the `on_dead` action.

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
│  │ (directory) │         │   (coop)    │                    │
│  └─────────────┘         └─────────────┘                    │
└─────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────┐
│  Adapters                                                   │
│  ┌───────────────────────────────────────────────────────┐  │
│  │                   RuntimeRouter                        │  │
│  │  LocalAdapter │ DockerAdapter │ KubernetesAdapter      │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

- **Job** creates and owns a **Workspace**
- **Agent** runs in a **Session** within that workspace
- **AgentAdapter** manages the agent lifecycle and process operations
- Session's `cwd` points to the workspace path (or an agent-specific `cwd` override)
- Multiple agents in a job share the same workspace
- **Worker** polls a **Queue** and dispatches items to jobs

## Summary

| Concept | Purpose | Implementation |
|---------|---------|----------------|
| **Workspace** | Isolated work directory | Local worktree or container volume |
| **Session** | Agent execution environment | Coop process (local or containerized) |
| **AgentAdapter** | Agent lifecycle management | RuntimeRouter → Local / Docker / K8s |

These abstractions enable the same runbook to work across different environments. The runbook defines *what* to do; the execution layer handles *where* and *how*.
