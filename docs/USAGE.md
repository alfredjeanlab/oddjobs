# Runbook Guide

Runbooks are written in HCL. Located in `.oj/runbooks/`.

## Writing Runbooks

### Minimal Example

```hcl
command "deploy" {
  args = "<env>"
  run  = { job = "deploy" }
}

job "deploy" {
  vars = ["env"]

  step "build" {
    run     = "make build"
    on_done = { step = "test" }
  }

  step "test" {
    run = "make test"
  }
}
```

## Key Patterns

**Locals** — computed once at job creation, available as `${local.*}`:

```hcl
locals {
  repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
  branch = "feature/${var.name}-${source.nonce}"
  title  = "feat(${var.name}): ${var.instructions}"
}
```

Use `local.repo` to resolve the main repo root when using `source = "folder"` with manual worktree commands.

**Job name templates** — human-readable names: `name = "${var.name}"`

**Notifications** — desktop alerts on lifecycle: `notify { on_start/on_done/on_fail }`

**Agent gates** — use `on_idle = { action = "gate", run = "make check" }` to verify
agent work via a shell command. No separate check step needed; the gate runs between
nudge cycles and controls step completion.

**Crons** — time-driven job execution:

```hcl
cron "janitor" {
  interval = "30m"
  run      = { job = "cleanup" }
}
```

Start with `oj cron start <name>`, test with `oj cron once <name>`.

**Prime** — inject context at agent session start to reduce tool calls.
Prime commands run as SessionStart hooks. Their output becomes context the
agent can reference immediately without running discovery commands itself.

Array form — cleaner for pure command sequences:

```hcl
agent "doctor" {
  prime = [
    "echo '## System Status'",
    "oj status 2>/dev/null || echo 'not running'",
    "echo '## Queues'",
    "oj queue list 2>/dev/null || echo 'no queues'",
  ]
}
```

Script form — natural for mixed static text and dynamic commands:

```hcl
agent "mayor" {
  prime = <<-SHELL
    cat <<'ROLE'
    ## Mayor — Project Coordinator
    You coordinate development across projects...
    ROLE

    echo '## Current Status'
    oj status 2>/dev/null || echo 'daemon not running'
  SHELL
}
```

**Persisted queues as inboxes** — queues aren't just for dispatch. Use them
to collect items for later human review:

```hcl
queue "treatments" {
  type = "persisted"
}
```

Agents push structured findings; humans review with `oj queue show`.

**Standalone agents** — commands can run agents directly without a job:

```hcl
command "mayor" {
  run = { agent = "mayor" }
}
```

Standalone agents are top-level WAL entities. They run in the invoking
directory (no workspace) and are visible via `oj agent list`.

**Crew** — long-lived standalone agents designed for ongoing, interactive
roles (coordinators, triagers, reviewers). Unlike job agents that
complete a task and exit, crew agents may idle waiting for user input.

Key patterns for crew agents:
- `on_idle = { action = "escalate" }` — notify the user instead of nudging
- Put the agent's role, responsibilities, and reference material in `prime`,
  not the prompt. Prime content anchors context summaries when the agent
  compacts. The prompt should be a simple initial instruction.
- Use `--disallowed-tools EnterPlanMode,ExitPlanMode` but allow
  AskUserQuestion so the agent can ask for clarification.

## Best Practices

**Shell:**
- `set -euo pipefail` is automatic
- Use newlines, not `&&` chains
- Use `test`, not `if` statements

**Agents:**
- Always `run = "claude --dangerously-skip-permissions"` (or `--model opus`)
- Set both `on_idle` and `on_dead` handlers
- Use gates (`on_idle = { action = "gate", run = "..." }`) to verify completion
- Keep prompts focused; the orchestrator handles lifecycle
- Use `--disallowed-tools EnterPlanMode,ExitPlanMode` for quick agents (fixes, chores, triage)
- For crew agents, put role/responsibilities in prime (survives compaction), keep prompt minimal
- Use prime to inject system state so agents start with context

**Steps:**
- `on_done = { step = "next" }` for explicit transitions
- `on_fail` for special handling (e.g. conflict resolution agent)
- `run = { agent = "name" }` to invoke agents from steps
- `on_fail = { step = "retry", attempts = 2 }` for bounded retry
- Combine independent shell commands in one step — don't chain separate steps
  for things that don't need individual error handling

**Commands:**
- `run = { job = "name" }` for job dispatch
- `run = { agent = "name" }` for standalone agents
- `run = <<-SHELL ... SHELL` for inline shell (e.g. launching crons, quick operations)

**Source:**
- `source { git = true }` for engine-managed git worktrees (init/cleanup handled automatically)
- `source = "folder"` for plain directories with manual git worktree setup (needed for custom start points)
- Share build cache: `.cargo/config.toml` → main repo's `target/` dir

**Workers and queues:**
- `queue` + `worker` for pull-based processing
- Queue types: `persisted` (internal) or `external` (backed by wok, etc.)
- Workers have `source`, `handler`, and `concurrency`
