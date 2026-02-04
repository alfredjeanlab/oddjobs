# Runbook Concepts

A runbook is a file that defines **commands** (user-facing entrypoints) and the **building blocks** they use (pipelines, agents, queues, and workers).

## Summary

```
┌─────────────────────────────────────────────────────────────┐
│ ENTRYPOINTS (things that run)                               │
│                                                             │
│   command ──► user invokes, runs pipeline or shell command  │
│   worker ───► polls a queue, dispatches items to pipelines  │
│   cron ─────► runs a pipeline on a recurring schedule       │
└─────────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│ BUILDING BLOCKS (composed by entrypoints)                   │
│                                                             │
│   pipeline ─► stepped execution                             │
│   agent ────► AI agent invocation                           │
│   queue ────► work items to be processed                    │
└─────────────────────────────────────────────────────────────┘
```

## Supported Formats

Runbook files placed in `.oj/runbooks/` are auto-discovered. Three formats are supported:

| Format | Extension | Example |
|--------|-----------|---------|
| HCL    | `.hcl`    | `build.hcl` |
| TOML   | `.toml`   | `build.toml` |
| JSON   | `.json`   | `build.json` |

All formats express the same primitives.

## Templates

All string fields in runbooks support two-step interpolation:

1. **Environment expansion**: `${VAR:-default}` expands from environment variables with fallback
2. **Variable substitution**: `${var.name}` expands from pipeline vars and context

Variable names support dotted notation (e.g., `${var.bug.title}`) and unknown variables are left as-is.

Available variable namespaces:

| Prefix | Source | Example |
|--------|--------|---------|
| `var.*` | Pipeline vars | `${var.bug.title}` |
| `args.*` | Command arguments | `${args.description}` |
| `item.*` | Queue item fields | `${item.id}` |
| `local.*` | Pipeline locals | `${local.repo}` |
| `workspace.*` | Workspace context | `${workspace.root}` |
| `invoke.*` | CLI invocation context | `${invoke.dir}` |

## Command

User-facing entrypoint. Accepts arguments, runs once.

```hcl
command "build" {
  args = "<name> <instructions>"
  run  = { pipeline = "build" }
}
```

Invoked: `oj run build auth "Add authentication"`

The `run` field specifies what to execute:
- Pipeline: `run = { pipeline = "build" }`
- Shell: `run = "echo hello"`

Commands also support a `defaults` map for default argument values:

```hcl
command "build" {
  args     = "<name> <instructions> [--base <branch>]"
  defaults = { base = "main" }
  run      = { pipeline = "build" }
}
```

### Argument Syntax

| Pattern | Meaning |
|---------|---------|
| `<name>` | Required positional |
| `[name]` | Optional positional |
| `<files...>` | Required variadic (1+) |
| `[files...]` | Optional variadic (0+) |
| `--flag` | Boolean flag |
| `-f/--flag` | Boolean flag with short alias |
| `--opt <val>` | Required flag with value |
| `[--opt <val>]` | Optional flag with value |
| `[-o/--opt <val>]` | Optional flag with value and short alias |

## Pipeline

Stepped execution with state tracking. Commands and workers invoke pipelines.

```hcl
pipeline "fix" {
  name      = "${var.bug.title}"
  vars      = ["bug"]
  workspace = "ephemeral"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "fix/${var.bug.id}-${workspace.nonce}"
    title  = "fix: ${var.bug.title}"
  }

  notify {
    on_start = "Fixing: ${var.bug.title}"
    on_done  = "Fix landed: ${var.bug.title}"
    on_fail  = "Fix failed: ${var.bug.title}"
  }

  step "fix" {
    run     = { agent = "bugfixer" }
    on_done = { step = "submit" }
  }

  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      git -C "${local.repo}" push origin "${local.branch}"
      oj queue push merges --var branch="${local.branch}" --var title="${local.title}"
    SHELL
  }
}
```

Pipeline fields:
- **name**: Optional name template for human-readable pipeline names (supports `${var.*}` interpolation; the result is slugified and suffixed with a unique nonce)
- **vars** (alias: `input`): List of required variable names
- **defaults**: Default values for vars
- **locals**: Map of local variables computed once at pipeline creation time (see [Locals](#locals) below)
- **cwd**: Base directory for execution (supports template interpolation)
- **workspace**: Workspace mode -- `"ephemeral"` (deleted on success, kept on failure) or `"persistent"` (never deleted)
- **notify**: Desktop notification templates for pipeline lifecycle (see [Desktop Integration](../interface/DESKTOP.md))
- **on_done**: Default step to route to when a step completes without an explicit `on_done`
- **on_fail**: Default step to route to when a step fails without an explicit `on_fail`
- **on_cancel**: Step to route to when the pipeline is cancelled (for cleanup)

### Name Templates

The optional `name` field provides a human-readable display name for pipeline instances. The template is interpolated with `${var.*}` variables, then slugified (lowercased, non-alphanumeric characters replaced with hyphens, stop words removed, truncated to 24 characters) and suffixed with a unique 8-character nonce.

```hcl
pipeline "build" {
  name = "${var.name}"
  # ...
}
```

`oj run build auth "Add authentication"` creates a pipeline displayed as `auth-a1b2c3d4` instead of `build-a1b2c3d4`.

### Locals

The `locals` block defines variables computed once at pipeline creation time. Local values support `${var.*}`, `${workspace.*}`, and `${invoke.*}` interpolation. Once evaluated, locals are available in all step templates as `${local.*}`.

```hcl
pipeline "build" {
  vars      = ["name", "instructions"]
  workspace = "ephemeral"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "feature/${var.name}-${workspace.nonce}"
    title  = "feat(${var.name}): ${var.instructions}"
  }

  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" HEAD
    SHELL
    on_done = { step = "work" }
  }
}
```

The `local.repo` pattern is particularly useful for ephemeral workspaces — it resolves the main repository root from the invocation directory, allowing steps to interact with the original repo (push branches, manage worktrees) while running inside an isolated workspace.

### Steps

The step `run` field specifies what to execute:
- Shell command: `run = "make check"`
- Agent reference: `run = { agent = "fix" }`
- Pipeline reference: `run = { pipeline = "deploy" }`

Step transitions use structured references:
- `on_done = { step = "next" }` -- next step on success
- `on_fail = { step = "recover" }` -- step to go to on failure

If `on_done` is omitted, the pipeline completes when the step succeeds. Steps without `on_fail` propagate failures up to the pipeline level.

## Agent

An AI agent invocation -- runs a recognized agent command in a monitored tmux session.

### Recognized Commands

| Command | Adapter |
|---------|---------|
| `claude` | `ClaudeAgentAdapter` |
| `claudeless` | `ClaudeAgentAdapter` |

Both commands route through the same adapter. See [Claude Code](../arch/06-adapter-claude.md) for integration details.

```hcl
agent "resolver" {
  run      = "claude --dangerously-skip-permissions"
  on_idle  = { action = "gate", run = "make check", attempts = 5 }
  on_dead  = { action = "escalate" }

  prompt = <<-PROMPT
    You are merging branch ${var.mr.branch} into ${var.mr.base}.
    Run `make check` to verify everything passes.
    When `make check` passes, say "I'm done"
  PROMPT
}
```

Agent fields:
- **run**: The agent command to execute (must be a recognized command)
- **prompt**: Inline prompt template (supports variable interpolation)
- **prompt_file**: Path to file containing prompt template (alternative to `prompt`)
- **env**: Map of environment variables to set
- **cwd**: Working directory (supports template interpolation)
- **prime**: Shell commands to run at session start for context injection (string or array)
- **on_idle**: What to do when agent is waiting for input (default: `"nudge"`)
- **on_dead** (alias: `on_exit`): What to do when agent process exits (default: `"escalate"`)
- **on_error**: What to do on API errors (default: `"escalate"`)

Valid actions per trigger:
- **on_idle**: `nudge`, `done`, `fail`, `escalate`, `gate`
- **on_dead**: `done`, `recover`, `fail`, `escalate`, `gate`
- **on_error**: `fail`, `escalate`, `gate`

Action options:
- **message**: Text for nudge (sent to session) or recover (modifies prompt)
- **append**: For recover -- `true` appends message to prompt, `false` (default) replaces it
- **run**: For gate -- shell command to run; exit 0 advances, non-zero escalates
- **attempts**: How many times to fire (default: 1; use `"forever"` for unlimited)
- **cooldown**: Delay between attempts (e.g., `"30s"`, `"5m"`)

The `on_error` field also supports per-error-type configuration:

```hcl
agent "fix" {
  on_error = [
    { match = "rate_limited", action = "gate", run = "sleep 60" },
    { action = "escalate" },
  ]
}
```

Supported error types: `unauthorized`, `out_of_credits`, `no_internet`, `rate_limited`.

### Prime (Context Injection)

The `prime` field runs shell commands at the start of an agent's session, injecting
their output as initial context. This is useful for providing git status, project
state, or other dynamic information that helps the agent orient itself.

**Script form** — a shell script (multi-line OK):

```hcl
agent "bugfixer" {
  prime = <<-SHELL
    echo '## Git'
    git branch --show-current
    git status --short | head -10
  SHELL
}
```

**Array form** — individual commands (each validated as a single shell command):

```hcl
agent "bugfixer" {
  prime = [
    "git branch --show-current",
    "git status --short | head -10",
  ]
}
```

Template variables (`${var.*}`, `${workspace}`, etc.) are interpolated before
the script is written. The prime script runs via a Claude Code `SessionStart`
hook, so its output appears as initial context in the agent's conversation.

### Inline Prompt via Template Variable

When using `prompt` or `prompt_file`, the run command must not include positional arguments (the system appends the prompt automatically). To embed the prompt inline in the command instead, use the `${prompt}` template variable:

```hcl
agent "inline" {
  run = "claude \"${prompt}\""
}
```

When `${prompt}` appears in the run command, the system interpolates the rendered prompt into the command string (with shell escaping) instead of appending it as a positional argument.

### Validation

The parser validates `agent.*.run` at parse time by extracting the first command name from the shell AST. Absolute paths are handled by taking the basename (e.g. `/usr/local/bin/claude` → `claude`). If the command name can't be statically determined (e.g. variable-only name or command substitution), validation is skipped silently.

Additional parse-time checks:
- **`--session-id` rejection**: The system adds `--session-id` automatically; including it in the run command is an error.
- **Positional argument rejection**: When `prompt` or `prompt_file` is configured, positional arguments in the run command are rejected (since the system appends the prompt).
- **Action-trigger compatibility**: Each action is validated against its trigger context (e.g. `recover` is invalid for `on_idle`; `nudge` is invalid for `on_dead`).

## Queue

A named collection of work items to be processed by a worker.

### Persisted Queue

Backed by oj's WAL state. Items are pushed via `oj queue push` and consumed by workers.

```hcl
queue "merges" {
  type     = "persisted"
  vars     = ["branch", "title", "base"]
  defaults = { base = "main" }
}
```

Push items via CLI:
```bash
oj queue push merges '{"branch": "fix-123", "title": "fix: button color"}'
```

The `vars` field declares required fields. `defaults` provides fallback values. Items are validated against the schema on push.

### Retry and Dead Letter

Persisted queues support automatic retry with dead letter semantics. When a pipeline fails after processing a queue item, the item can be retried automatically before being moved to a terminal `Dead` status.

```hcl
queue "bugs" {
  type = "persisted"
  vars = ["id", "title"]
  retry = {
    attempts = 3       # Number of auto-retry attempts (0 = no retry, default)
    cooldown = "30s"   # Delay between retries (default: "0s")
  }
}
```

- **attempts**: How many times to retry before marking dead (default: 0 — failed items go directly to dead)
- **cooldown**: Delay between retry attempts (e.g., `"30s"`, `"5m"`)

With no retry configuration (the default), failed items go directly to `Dead` status. Dead or failed items can be manually retried with `oj queue retry <queue> <item-id>`.

The `retry` block is only valid on persisted queues; external queues reject it at parse time.

### External Queue

Backed by an external system, polled via shell commands.

```hcl
queue "bugs" {
  type = "external"
  list = "wok list -t bug -s todo --unassigned -o json"
  take = "wok start ${item.id}"
}
```

- **list**: Shell command that returns a JSON array of items
- **take**: Shell command to claim an item (supports `${item.*}` interpolation)

## Worker

Polls a queue and dispatches each item to a pipeline for processing.

```hcl
worker "merge" {
  source      = { queue = "merges" }
  handler     = { pipeline = "merge" }
  concurrency = 1
}
```

Worker fields:
- **source**: Which queue to consume from (`{ queue = "name" }`)
- **handler**: Which pipeline to run per item (`{ pipeline = "name" }`)
- **concurrency**: Maximum concurrent pipeline instances (default: 1)

Workers are started via `oj worker start <name>`. The command is idempotent — if the worker is already running, it wakes it to poll immediately.

When a worker takes an item from the queue, the item's fields are mapped into the pipeline's first declared var as a namespace. For example, if the pipeline declares `vars = ["mr"]` and the queue item has `{"branch": "fix-123"}`, the pipeline receives `var.mr.branch = "fix-123"`.

## Cron

Time-driven entrypoint. Runs a pipeline on a recurring schedule.

```hcl
cron "janitor" {
  interval    = "30m"
  run         = { pipeline = "cleanup" }
  concurrency = 1
}
```

Cron fields:
- **interval**: How often to run (e.g., `"30m"`, `"6h"`, `"24h"`)
- **run**: What to execute (`{ pipeline = "name" }`)
- **concurrency**: Maximum concurrent pipeline instances (default: 1 — singleton)

Crons are the third entrypoint type alongside commands and workers:

```text
User ─── oj run ───► Command ───► Pipeline (direct)
Queue ──────────────► Worker ────► Pipeline (background)
Timer ──────────────► Cron ──────► Pipeline (scheduled)
```

Managed via `oj cron start <name>`, `oj cron stop <name>`, `oj cron once <name>`. Use cases range from simple shell-step cleanup (janitor) to agent-driven periodic analysis.

## Recovery

Agent lifecycle actions handle different states:

| Action | Effect |
|--------|--------|
| `nudge` | Send message prompting agent to continue |
| `done` | Treat as success, advance pipeline |
| `fail` | Mark pipeline as failed |
| `recover` | Re-spawn agent with modified prompt |
| `escalate` | Alert for human intervention |
| `gate` | Run a shell command; advance if exit 0, escalate otherwise |

## File Organization

Each runbook file defines related primitives:

| File | Defines | Description |
|------|---------|-------------|
| `build.hcl` | command, pipeline, agents | Feature development: plan, execute, merge |
| `bugfix.hcl` | command, pipeline, queue, worker, agent | Bug fix workflow with worker pool |
| `merge/local.hcl` | queue, worker, pipeline, agent | Local merge queue with conflict resolution |
| `merge/github.hcl` | queue, worker, pipeline | GitHub PR merge queue |
| `maintenance.hcl` | cron, pipeline | Scheduled cleanup and maintenance tasks |

Primitives are referenced by name within a runbook.
