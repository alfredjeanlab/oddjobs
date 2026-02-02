# Runbooks

Runbooks are written in HCL.

## Available Runbooks

### runbooks/build.hcl
Feature development: init → plan agent → implement agent → submit

```bash
oj run build my-feature "Add user authentication"
```

### runbooks/bugfix.hcl
Bug worker pool: pulls bugs from wok → fix agent → submit → done

```bash
oj run fix "Button doesn't respond to clicks"
oj worker start fix
```

### runbooks/merge.hcl
Local merge queue: merge → check → push (with conflict resolution agent)

```bash
oj queue push merges '{"branch": "fix-123", "title": "fix: button color"}'
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

agent "reviewer" {
  run      = "claude --dangerously-skip-permissions"
  on_idle  = { action = "nudge", message = "Keep working." }
  on_dead  = { action = "gate", run = "make check" }
  prompt   = "Review the code."
}
```

## Best Practices

**Shell scripts:**
- `set -e` is automatic — commands fail on error
- Use newlines, not `&&` chains
- Use `test` command, not `if` statements

**Agents:**
- Always use `run = "claude --dangerously-skip-permissions"`
- Set `on_idle` (nudge/done/fail/escalate/gate) and `on_dead` (done/fail/recover/escalate/gate)
- Keep prompts focused on the task; the orchestrator handles completion

**Steps:**
- Use `on_done = { step = "next" }` for explicit transitions
- Use `on_fail` only for special handling (like conflict resolution)
- Use `run = { agent = "name" }` to invoke agents from pipeline steps

**Workspaces:**
- Use `workspace = "ephemeral"` for isolated git worktrees
- Share build cache via `.cargo/config.toml` pointing at the main repo's target dir

**Workers and queues:**
- Use `queue` + `worker` for pull-based processing
- Queue types: `persisted` (internal) or `external` (backed by external tool)
- Workers have `source`, `handler`, and `concurrency`
