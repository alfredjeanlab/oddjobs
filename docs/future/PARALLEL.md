# CI/CD Execution Model (V1)

Parallel step execution and per-step runtimes.

## Current Model

Jobs contain sequential steps. One step is active at a time. Steps transition
via `on_done`/`on_fail`/`on_cancel` — a state machine, not a linear pipeline.
Steps share a workspace (git worktree or folder), which serves as the
data-passing mechanism between steps.

## 1. Parallel Steps

### Fan-Out: Array `on_done`

`on_done` accepts an array to start multiple steps simultaneously:

```hcl
step "build" {
  run     = "cargo build --release"
  on_done = [{ step = "test-unit" }, { step = "test-integration" }]
}

step "test-unit" {
  run = "cargo test --lib"
}

step "test-integration" {
  run = "cargo test --test integration"
}
```

Both test steps start when build completes. Each runs independently.

### Fan-In: `after`

A step with `after` waits for all listed steps to complete before starting:

```hcl
step "deploy" {
  after = ["test-unit", "test-integration"]
  run   = "make deploy"
}
```

`after` is pull-based — the step declares what it's waiting for. Reads
naturally: "deploy runs after test-unit and test-integration."

### Full Example

```hcl
step "build" {
  run     = "cargo build --release"
  on_done = [{ step = "test-unit" }, { step = "test-integration" }]
}

step "test-unit"        { run = "cargo test --lib" }
step "test-integration" { run = "cargo test --test integration" }

step "deploy" {
  after = ["test-unit", "test-integration"]
  run   = "make deploy"
}
```

### Validation: No Conflicting Activation

A step is either **push-activated** (someone's `on_done` target) or
**pull-activated** (`after`). Never both.

**Rule**: A step with `after` cannot appear as a `{ step = "..." }` target in
any `on_done` or `on_fail`. The parser rejects this at load time.

```hcl
# PARSE ERROR: deploy has `after` but is also test-unit's on_done target
step "test-unit" {
  on_done = { step = "deploy" }         # ← rejected
}

step "deploy" {
  after = ["test-unit", "test-integration"]
  run   = "make deploy"
}
```

The fix is clear: remove `on_done` from test-unit. The `after` on deploy is
the sole activation mechanism.

Today's state machine convergence (two `on_done` paths reaching the same step,
but never simultaneously) still works — that's push-activated from multiple
sources, which is fine because only one is ever active. The validation only
rejects mixing push and pull on the same step.

### Failure Handling

When a parallel branch fails, the `after` step fails too (prerequisites not
met). Remaining branches are cancelled. The job follows the failing step's
`on_fail`, or the job-level `on_fail`.

### Implementation

- `on_done` field type: `OneOrMany<StepTransition>` (single or array)
- Job state: `active_steps: HashSet<String>` replaces `step: String`
- Per-step status: `step_statuses: HashMap<String, StepStatus>`
- Completion tracker: `pending_after: HashMap<String, HashSet<String>>`
  — for each `after` step, which predecessors haven't completed yet
- When a step completes, check if any `after` steps become unblocked
- Parser validation: `after` steps cannot be `on_done`/`on_fail` targets

## 2. Per-Step Runtime

Different steps can run on different platforms. Extends the `runtime` concept
from CLOUD_V1 to the step level.

```hcl
step "test-linux" {
  runtime "docker" { image = "ubuntu:24.04" }
  run = "cargo test"
}

step "test-macos" {
  runtime = "local"
  run = "cargo test"
}
```

Resolution order: `step > agent > job > project default > "local"`.

### Implementation

- Add `runtime` field to step definitions (optional, inherits)
- Effects carry runtime config
- `RuntimeRouter` (from CLOUD_V1) dispatches to appropriate adapter

## Composition

```hcl
job "release" {
  source { git = true }

  step "build" {
    run     = "cargo build --release"
    on_done = [{ step = "test-linux" }, { step = "test-macos" }]
  }

  step "test-linux" {
    runtime "docker" { image = "ubuntu:24.04" }
    run = "cargo test"
  }

  step "test-macos" {
    runtime = "local"
    run = "cargo test"
  }

  step "publish" {
    after = ["test-linux", "test-macos"]
    run   = "gh release create v$(cargo metadata --format-version=1 | jq -r '.packages[0].version')"
  }
}
```

## Future Extensions

**Step outputs**: Formal `${step.build.stdout}` interpolation for cross-step
data when the shared workspace model is insufficient (e.g., steps on different
machines without shared storage).

**Matrix expansion**: Sugar that generates parallel steps with different
variables. Lowers to array `on_done` + generated steps + `after` join.

**Conditional steps**: `when` guard for conditional execution.
