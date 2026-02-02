# Pipeline Locals

## Overview

Add a `locals {}` block to pipeline definitions that allows defining computed key-value pairs at the pipeline level. Locals are evaluated once at pipeline creation time and made available as `${local.*}` in step `run` blocks, agent prompts, and other interpolation contexts. They can reference `${var.*}`, `${invoke.*}`, `${workspace.*}`, and other template expressions available at creation time.

## Project Structure

Files to modify:

```
crates/
├── runbook/
│   ├── src/
│   │   ├── pipeline.rs          # Add `locals` field to PipelineDef
│   │   ├── pipeline_tests.rs    # Tests for locals parsing
│   │   ├── parser.rs            # (no changes needed — serde handles it)
│   │   └── parser_tests/mod.rs  # Tests for locals in HCL/TOML parsing
├── engine/
│   ├── src/
│   │   └── runtime/
│   │       ├── handlers/
│   │       │   └── pipeline_create.rs  # Evaluate locals, inject into vars
│   │       └── pipeline.rs             # Expose local.* in step interpolation
│   └── src/
│       └── spawn.rs                    # Expose local.* in agent prompt interpolation
.oj/runbooks/
├── bugfix.hcl                   # Use locals for repo, branch
└── build.hcl                    # Use locals for repo, branch, title
```

## Dependencies

No new external dependencies. Uses existing:
- `hcl` crate (already supports arbitrary block attributes)
- `serde` (HashMap deserialization)
- `oj_runbook::interpolate` / `interpolate_shell` (template expansion)

## Implementation Phases

### Phase 1: Add `locals` field to `PipelineDef`

Add an optional `locals` field to the `PipelineDef` struct in `crates/runbook/src/pipeline.rs`.

```rust
// In PipelineDef:
/// Local variables computed at pipeline creation time.
/// Values are template strings evaluated once, available as ${local.*}.
#[serde(default)]
pub locals: HashMap<String, String>,
```

HCL `locals {}` blocks deserialize as a map via serde — no custom parser code needed. The `hcl` crate treats `locals { key = "value" }` the same as `locals = { key = "value" }`.

**Verification:** Write a unit test in `pipeline_tests.rs` that parses a pipeline with `locals` from HCL and confirms the values are present in the struct.

### Phase 2: Evaluate locals during pipeline creation

In `crates/engine/src/runtime/handlers/pipeline_create.rs`, after workspace variables are injected (line 87) and before the `PipelineCreated` event is emitted (line 130), evaluate each local's template string and insert the results into the vars map with `local.` prefix.

```rust
// After workspace.* vars are injected (line 87):

// Evaluate locals: interpolate each value with current vars, then add to vars
for (key, template) in &pipeline_def.locals {
    let value = oj_runbook::interpolate(template, &vars);
    vars.insert(format!("local.{}", key), value);
}
```

**Key decisions:**
- Locals are evaluated with the same `vars` map that contains `var.*`, `workspace.*`, and `invoke.*` — no separate context needed.
- Locals use `interpolate` (not `interpolate_shell`) since they are raw values, not shell commands. Shell escaping happens later when the local is substituted into a shell step.
- Locals do NOT reference other locals — each local sees only `var.*`, `workspace.*`, and `invoke.*`. This avoids ordering complexity. If a local template references `${local.x}`, it will be left unexpanded (standard behavior for unknown variables).
- Evaluated locals flow into the `PipelineCreated` event's `vars` map, so they are persisted and available on daemon restart.

**Verification:** Write an engine test that creates a pipeline with locals referencing `${var.*}` and `${workspace.*}`, and confirms the evaluated `local.*` entries appear in the pipeline's vars.

### Phase 3: Expose `local.*` in step and agent interpolation

The step runner (`crates/engine/src/runtime/pipeline.rs:50-73`) and agent spawner (`crates/engine/src/spawn.rs:68-89`) both build a `vars` HashMap from `input` (which is `pipeline.vars`). They namespace input keys under `var.` prefix and selectively expose `workspace.*` and `invoke.*` at top level.

Add `local.*` to the set of keys exposed at top level in both locations:

**In `pipeline.rs` (shell steps), after the `workspace.*`/`invoke.*` loop (~line 64):**

```rust
// Expose local.* variables for shell interpolation
for (key, val) in input.iter() {
    if key.starts_with("local.") {
        vars.insert(key.clone(), val.clone());
    }
}
```

**In `spawn.rs` (agent prompts), after `invoke.dir` insertion (~line 89):**

```rust
// Expose local.* variables for prompt interpolation
for (key, val) in input.iter() {
    if key.starts_with("local.") {
        prompt_vars.insert(key.clone(), val.clone());
    }
}
```

**Verification:** Confirm that `${local.branch}` in a shell step `run` block and in an agent `prompt` both expand correctly.

### Phase 4: Update runbooks to use locals

Update `bugfix.hcl` and `build.hcl` to use locals for `repo`, `branch`, and `title`, eliminating duplicated shell variable assignments.

**bugfix.hcl:**

```hcl
pipeline "fix" {
  vars      = ["bug"]
  workspace = "ephemeral"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "fix/${var.bug.id}-${workspace.nonce}"
    title  = "fix: ${var.bug.title}"
  }

  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" HEAD
      mkdir -p .cargo
      echo "[build]" > .cargo/config.toml
      echo "target-dir = \"${local.repo}/target\"" >> .cargo/config.toml
    SHELL
    on_done = { step = "fix" }
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
      oj worker start merge
    SHELL
    on_done = { step = "done" }
  }

  step "done" {
    run = "cd ${invoke.dir} && wok done ${var.bug.id}"
  }
}
```

**build.hcl** — same pattern: extract `repo`, `branch`, and `title` into locals.

```hcl
pipeline "build" {
  vars = ["name", "instructions", "base", "rebase", "new"]
  workspace = "ephemeral"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "feature/${var.name}-${workspace.nonce}"
    title  = "feat(${var.name}): ${var.instructions}"
  }

  # ... steps use ${local.repo}, ${local.branch}, ${local.title}
}
```

Note: `local.repo` contains a shell command substitution `$(...)`. This is fine — locals are template-interpolated (expanding `${var.*}` etc.), but the `$(...)` is left as literal text. When the local value is later substituted into a shell `run` block, the shell evaluates `$(...)` at execution time.

**Verification:** Run `oj run fix <bug>` and `oj run build <name> <instructions>` end-to-end and confirm the pipelines work correctly with locals.

## Key Implementation Details

### Evaluation semantics

- Locals are evaluated **once** at pipeline creation time using `oj_runbook::interpolate()`.
- They can reference `${var.*}`, `${workspace.*}`, `${invoke.*}`, and `${VAR:-default}` env vars.
- They do **not** reference other locals (no `${local.*}` during evaluation). This keeps the implementation simple — no dependency ordering or cycle detection needed.
- Unknown variables are left as-is (existing behavior of `interpolate()`).

### Data flow

```
HCL parse → PipelineDef.locals: HashMap<String, String> (raw templates)
    ↓
pipeline_create.rs: interpolate each local with current vars
    ↓
Insert into vars as "local.key" → stored in PipelineCreated event
    ↓
pipeline.rs / spawn.rs: locals flow through input map, exposed as local.*
    ↓
Shell steps: interpolate_shell() expands ${local.*}
Agent prompts: interpolate() expands ${local.*}
```

### Shell command substitution in locals

Locals like `repo = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"` work because:
1. At pipeline creation: `interpolate()` expands `${invoke.dir}` → `"/path/to/dir"`, yielding `"$(git -C /path/to/dir rev-parse --show-toplevel)"`
2. This literal string (with `$(...)`) is stored in vars as `local.repo`
3. At step execution: `interpolate_shell()` substitutes `${local.repo}` into the shell command
4. The shell then evaluates `$(git -C /path/to/dir rev-parse --show-toplevel)` at runtime

This means `local.repo` is **not** the resolved path — it's a shell expression that resolves at each step's execution. This is the correct behavior: it keeps locals as template-level substitutions, not execution-time evaluations.

### No parser changes needed

The `hcl` crate's serde integration handles `locals { key = "val" }` as a map automatically. Adding `locals: HashMap<String, String>` with `#[serde(default)]` to `PipelineDef` is sufficient. No changes to `parser.rs` validation are needed — locals values are opaque strings at parse time.

## Verification Plan

1. **Unit tests (Phase 1):** Parse HCL with `locals {}` block, verify `PipelineDef.locals` contains expected key-value pairs
2. **Unit tests (Phase 2):** Create a pipeline with locals that reference `${var.x}` and `${workspace.nonce}`, verify the vars map contains evaluated `local.*` entries
3. **Unit tests (Phase 3):** Verify `${local.*}` expands in both shell step commands and agent prompts
4. **`make check`:** `cargo fmt`, `cargo clippy`, `cargo test --all`, `cargo build --all`
5. **Manual verification:** Run updated `bugfix.hcl` and `build.hcl` runbooks to confirm end-to-end behavior
