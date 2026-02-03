# Runbooks

Runbooks are written in HCL. Located in `.oj/runbooks/`.

## Available Commands

### build.hcl — Feature development
init → plan agent → implement agent → submit to merge queue

```bash
oj run build my-feature "Add user authentication"
```

### bug.hcl — Bug fixes (worker pool)
Files bug in wok, worker pulls → fix agent → submit → mark done

```bash
oj run fix "Button doesn't respond to clicks"
```

### chore.hcl — Chores (worker pool)
Files chore in wok, worker pulls → agent → submit → mark done

```bash
oj run chore "Update dependencies to latest versions"
```

### draft.hcl — Exploratory work
Pushed to `draft/<name>` branches, not merged.

```bash
oj run draft inline-commands "Execute shell commands locally"
oj run draft-rebase inline-commands   # rebase draft onto main
oj run drafts                         # list open drafts
oj run drafts --close inline-commands # delete a draft branch
```

### merge.hcl — Local merge queue
merge → check → push, with agent conflict resolution

```bash
oj run merge feature/auth-abc123 "feat: add authentication"
oj worker start merge
```

## Writing Runbooks

### Minimal Example

```hcl
command "deploy" {
  args = "<env>"
  run  = { pipeline = "deploy" }
}

pipeline "deploy" {
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

**Locals** — computed once at pipeline creation, available as `${local.*}`:

```hcl
locals {
  repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
  branch = "feature/${var.name}-${workspace.nonce}"
  title  = "feat(${var.name}): ${var.instructions}"
}
```

Use `local.repo` to resolve the main repo root from an ephemeral worktree.

**Pipeline name templates** — human-readable names: `name = "${var.name}"`

**Notifications** — desktop alerts on lifecycle: `notify { on_start/on_done/on_fail }`

**Agent gates** — use `on_idle = { action = "gate", run = "make check" }` to verify
agent work via a shell command. No separate check step needed; the gate runs between
nudge cycles and controls step completion.

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

**Steps:**
- `on_done = { step = "next" }` for explicit transitions
- `on_fail` for special handling (e.g. conflict resolution agent)
- `run = { agent = "name" }` to invoke agents from steps
- `on_fail = { step = "retry", attempts = 2 }` for bounded retry

**Workspaces:**
- `workspace = "ephemeral"` for isolated git worktrees
- Share build cache: `.cargo/config.toml` → main repo's `target/` dir
- Always add a cleanup step: `git worktree remove --force`

**Workers and queues:**
- `queue` + `worker` for pull-based processing
- Queue types: `persisted` (internal) or `external` (backed by wok, etc.)
- Workers have `source`, `handler`, and `concurrency`
