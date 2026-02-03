# Runbook Validation Plan

## Overview

Add semantic validation to runbook parsing that catches invalid name references and structural issues at parse time. This includes rejecting dangling step/agent/queue references, detecting duplicate names within a runbook, and warning on unreachable or dead-end steps. All validation runs inside `parse_runbook_with_format()` in `crates/runbook/src/parser.rs`, keeping the single-runbook parse boundary. Cross-runbook duplicate detection is added to the `find.rs` collection functions.

## Project Structure

All changes are in `crates/runbook/src/`:

```
crates/runbook/src/
├── parser.rs              # Main validation logic (new steps 8-10 in parse_runbook_with_format)
├── parser_tests/
│   ├── mod.rs             # Existing tests
│   └── references.rs      # NEW: tests for reference validation + reachability warnings
├── find.rs                # Cross-runbook duplicate detection in collect_* functions
└── find_tests.rs          # Tests for cross-runbook duplicates
```

## Dependencies

No new dependencies required. Existing crates (`thiserror`, `tracing`, `indexmap`) suffice.

## Implementation Phases

### Phase 1: Step Reference Validation

Validate that `on_done`, `on_fail`, and `on_cancel` step transitions reference steps that actually exist in the pipeline.

**Where:** `parser.rs`, new validation block after step 3 (step name validation, ~line 332) or as a new step 8 after the existing step 7.

**Logic:** For each pipeline, collect step names into a `HashSet`. Then iterate all steps checking each `on_done`/`on_fail`/`on_cancel` `StepTransition`. Also check the pipeline-level `on_done`/`on_fail`/`on_cancel`. Return `ParseError::InvalidFormat` if a referenced step doesn't exist.

```rust
// Step 8: Validate step transition references
for (pipeline_name, pipeline) in &runbook.pipelines {
    let step_names: HashSet<&str> = pipeline.steps.iter().map(|s| s.name.as_str()).collect();

    // Check pipeline-level transitions
    for (field, transition) in [
        ("on_done", &pipeline.on_done),
        ("on_fail", &pipeline.on_fail),
        ("on_cancel", &pipeline.on_cancel),
    ] {
        if let Some(t) = transition {
            if !step_names.contains(t.step_name()) {
                return Err(ParseError::InvalidFormat {
                    location: format!("pipeline.{}.{}", pipeline_name, field),
                    message: format!(
                        "references unknown step '{}'; available steps: {}",
                        t.step_name(),
                        step_names.iter().copied().collect::<Vec<_>>().join(", "),
                    ),
                });
            }
        }
    }

    // Check step-level transitions
    for (i, step) in pipeline.steps.iter().enumerate() {
        for (field, transition) in [
            ("on_done", &step.on_done),
            ("on_fail", &step.on_fail),
            ("on_cancel", &step.on_cancel),
        ] {
            if let Some(t) = transition {
                if !step_names.contains(t.step_name()) {
                    return Err(ParseError::InvalidFormat {
                        location: format!("pipeline.{}.step[{}]({}).{}", pipeline_name, i, step.name, field),
                        message: format!(
                            "references unknown step '{}'; available steps: {}",
                            t.step_name(),
                            step_names.iter().copied().collect::<Vec<_>>().join(", "),
                        ),
                    });
                }
            }
        }
    }
}
```

**Tests:** Parse a runbook with `on_done = "nonexistent"` and assert `ParseError::InvalidFormat`. Parse valid transitions and assert success. Test pipeline-level and step-level transitions separately.

**Milestone:** `cargo test -p oj-runbook` passes with new tests for step reference validation.

### Phase 2: Agent and Queue Reference Validation

Validate that `run = { agent = "name" }` references an agent defined in the same runbook, and that `source = { queue = "name" }` references a defined queue (the worker queue check already exists; this adds validation for any other queue references if applicable).

**Where:** `parser.rs`, new validation block (step 9).

**Logic:**
- For each pipeline step and command with `RunDirective::Agent { agent }`, check `runbook.agents.contains_key(agent)`.
- For each pipeline step and command with `RunDirective::Pipeline { pipeline }`, check `runbook.pipelines.contains_key(pipeline)` (bonus — same pattern, low cost).
- The worker queue/pipeline validation already exists in step 6. No changes needed there.

```rust
// Step 9: Validate agent and pipeline references in steps and commands
for (pipeline_name, pipeline) in &runbook.pipelines {
    for (i, step) in pipeline.steps.iter().enumerate() {
        if let Some(agent_name) = step.run.agent_name() {
            if !runbook.agents.contains_key(agent_name) {
                return Err(ParseError::InvalidFormat {
                    location: format!("pipeline.{}.step[{}]({}).run", pipeline_name, i, step.name),
                    message: format!(
                        "references unknown agent '{}'; available agents: {}",
                        agent_name,
                        runbook.agents.keys().cloned().collect::<Vec<_>>().join(", "),
                    ),
                });
            }
        }
        if let Some(pl_name) = step.run.pipeline_name() {
            if !runbook.pipelines.contains_key(pl_name) {
                return Err(ParseError::InvalidFormat {
                    location: format!("pipeline.{}.step[{}]({}).run", pipeline_name, i, step.name),
                    message: format!(
                        "references unknown pipeline '{}'; available pipelines: {}",
                        pl_name,
                        runbook.pipelines.keys().cloned().collect::<Vec<_>>().join(", "),
                    ),
                });
            }
        }
    }
}

for (cmd_name, cmd) in &runbook.commands {
    if let Some(agent_name) = cmd.run.agent_name() {
        if !runbook.agents.contains_key(agent_name) {
            return Err(ParseError::InvalidFormat {
                location: format!("command.{}.run", cmd_name),
                message: format!(
                    "references unknown agent '{}'; available agents: {}",
                    agent_name,
                    runbook.agents.keys().cloned().collect::<Vec<_>>().join(", "),
                ),
            });
        }
    }
    if let Some(pl_name) = cmd.run.pipeline_name() {
        if !runbook.pipelines.contains_key(pl_name) {
            return Err(ParseError::InvalidFormat {
                location: format!("command.{}.run", cmd_name),
                message: format!(
                    "references unknown pipeline '{}'; available pipelines: {}",
                    pl_name,
                    runbook.pipelines.keys().cloned().collect::<Vec<_>>().join(", "),
                ),
            });
        }
    }
}
```

**Note:** `RunDirective` already has `agent_name()` accessor. A `pipeline_name()` accessor may need to be added (check if it exists; if not, add a simple match arm).

**Tests:** Runbook with `run = { agent = "ghost" }` where no agent "ghost" is defined → error. Same for pipeline references.

**Milestone:** `cargo test -p oj-runbook` passes with agent/pipeline reference tests.

### Phase 3: Duplicate Name Detection (Within a Runbook)

Detect duplicate names within a single runbook. Since `Runbook` uses `HashMap<String, _>` for all entity maps, serde silently last-wins on duplicates. The fix is to detect collisions *across* entity types (e.g., a step named "build" in two different pipelines is fine, but two agents named "coder" is caught by HashMap — the real gap is cross-type collisions if needed, and duplicate step names within a single pipeline).

**Where:** `parser.rs`, new validation block (step 10).

**Logic:**
- For each pipeline, check for duplicate step names: collect step names and check for repeats.
- Cross-type duplicates (e.g., agent and command with same name) — evaluate if this matters semantically. Since entities are namespaced by type in HCL (`agent "x"` vs `command "x"`), cross-type duplicates are likely fine. Focus on **within-type** duplicates that HashMap silently swallows.

**Approach for within-type HashMap duplicates:** Since serde deserializes into `HashMap` which silently overwrites, detecting true duplicates requires either:
1. A custom deserializer wrapper, or
2. Post-parse step name checking (only applicable to `Vec<StepDef>` in pipelines since steps are ordered).

For `HashMap`-based entities (commands, agents, queues, pipelines, workers), serde already deduplicates. The pragmatic approach: just validate duplicate step names within each pipeline (steps are `Vec<StepDef>`, not a map).

```rust
// Step 10: Detect duplicate step names within pipelines
for (pipeline_name, pipeline) in &runbook.pipelines {
    let mut seen = HashSet::new();
    for (i, step) in pipeline.steps.iter().enumerate() {
        if !seen.insert(step.name.as_str()) {
            return Err(ParseError::InvalidFormat {
                location: format!("pipeline.{}.step[{}]({})", pipeline_name, i, step.name),
                message: format!("duplicate step name '{}'", step.name),
            });
        }
    }
}
```

**Tests:** Pipeline with two steps both named "deploy" → error.

**Milestone:** `cargo test -p oj-runbook` passes with duplicate step name tests.

### Phase 4: Cross-Runbook Duplicate Detection

Detect entities defined in multiple runbook files when collecting all definitions.

**Where:** `find.rs`, in `collect_all_commands()` and `collect_all_queues()`. Also add `collect_all_agents()` and `collect_all_pipelines()` if needed, or a general `validate_all_runbooks()` function.

**Logic:** As entities are collected across files, track which file each name came from. If a name appears in two different files, return `FindError::Duplicate`.

```rust
// In collect_all_commands, after collecting:
let mut seen: HashMap<&str, &Path> = HashMap::new();
for (name, _cmd, path) in &commands {
    if let Some(prev_path) = seen.insert(name.as_str(), path) {
        return Err(FindError::Duplicate(name.clone()));
    }
}
```

**Note:** The collect functions currently don't track source paths. They'll need to carry `(String, EntityDef, PathBuf)` tuples internally, or check for duplicates during collection.

Extend this to agents and pipelines by either:
- Adding `collect_all_agents()` / `collect_all_pipelines()` / `collect_all_workers()`, or
- Adding a comprehensive `validate_runbook_dir()` function that scans all files and checks all entity types for cross-file duplicates.

The comprehensive function is preferred since it avoids redundant file scanning:

```rust
/// Validate all runbooks in a directory for cross-file conflicts.
/// Returns errors for any entity name defined in multiple files.
pub fn validate_runbook_dir(runbook_dir: &Path) -> Result<(), Vec<FindError>> { ... }
```

**Tests:** Two runbook files both defining `command "deploy"` → `FindError::Duplicate`. Two files defining agents with the same name → error. Same name in different entity types → OK.

**Milestone:** `cargo test -p oj-runbook` passes with cross-runbook duplicate tests.

### Phase 5: Unreachable and Dead-End Step Warnings

Warn (not error) on steps that are unreachable or lack transitions.

**Where:** `parser.rs`. Since `parse_runbook_with_format` returns `Result<Runbook, ParseError>`, warnings need a different channel. Options:
1. Use `tracing::warn!()` — simplest, consistent with existing pattern in `find.rs`.
2. Add a `warnings: Vec<String>` field to `Runbook` — more structured but changes the API.

**Recommended:** Use `tracing::warn!()` for simplicity. This matches the project's existing approach (find.rs already logs warnings for skipped files).

**Unreachable step detection:**
- Step 0 (first step) is always reachable.
- A step is reachable if it's referenced by any `on_done`, `on_fail`, or `on_cancel` of another step, or by pipeline-level `on_done`/`on_fail`/`on_cancel`.
- Collect all referenced step names, then warn on any step (except index 0) not in the set.

```rust
for (pipeline_name, pipeline) in &runbook.pipelines {
    if pipeline.steps.len() <= 1 {
        continue;
    }
    let mut referenced: HashSet<&str> = HashSet::new();
    // Collect from pipeline-level transitions
    for t in [&pipeline.on_done, &pipeline.on_fail, &pipeline.on_cancel].iter().flatten() {
        referenced.insert(t.step_name());
    }
    // Collect from step-level transitions
    for step in &pipeline.steps {
        for t in [&step.on_done, &step.on_fail, &step.on_cancel].iter().flatten() {
            referenced.insert(t.step_name());
        }
    }
    // Warn on unreachable (skip first step)
    for step in pipeline.steps.iter().skip(1) {
        if !referenced.contains(step.name.as_str()) {
            tracing::warn!(
                "pipeline.{}: step '{}' is unreachable (not referenced by any on_done/on_fail/on_cancel)",
                pipeline_name, step.name
            );
        }
    }
}
```

**Dead-end step detection:**
- A step is a dead-end if it has no `on_done` and is not the last step in the list.
- The last step naturally completes the pipeline, so lacking `on_done` is expected.
- Steps with `on_fail` but no `on_done` are still dead-ends for the success path.

```rust
for (pipeline_name, pipeline) in &runbook.pipelines {
    let last_idx = pipeline.steps.len().saturating_sub(1);
    for (i, step) in pipeline.steps.iter().enumerate() {
        if i < last_idx && step.on_done.is_none() {
            tracing::warn!(
                "pipeline.{}: step '{}' has no on_done and is not the last step (implicit sequential advance)",
                pipeline_name, step.name
            );
        }
    }
}
```

**Important consideration:** Check whether the engine implicitly advances to the next step when `on_done` is `None`. If so, lacking `on_done` on non-last steps is normal (implicit sequential flow) and the warning should be adjusted or removed. Investigate `crates/engine/` to confirm pipeline step advancement semantics before deciding if this warning is appropriate.

**Tests:** Use `tracing-subscriber` test utilities or a `tracing` mock layer to capture warnings in tests, or test the logic in a helper function that returns the warnings as a `Vec<String>`.

**Milestone:** `cargo test -p oj-runbook` passes; warnings emitted for unreachable/dead-end steps in test runbooks.

## Key Implementation Details

1. **Validation ordering:** New validations (steps 8-10) go after existing step 7 (action-trigger compatibility) in `parse_runbook_with_format()`. Step reference validation (phase 1) must come after duplicate step name detection (phase 3) to avoid confusing errors. Reorder so phase 3 runs first in the final code.

2. **`RunDirective::pipeline_name()` accessor:** If this method doesn't exist on `RunDirective`, add it as a simple match:
   ```rust
   pub fn pipeline_name(&self) -> Option<&str> {
       match self {
           RunDirective::Pipeline { pipeline } => Some(pipeline),
           _ => None,
       }
   }
   ```

3. **Error message style:** Follow the existing pattern from worker validation (lines 427-458 of `parser.rs`): `"references unknown {type} '{name}'; available {types}: {list}"`.

4. **`HashSet` import:** Add `use std::collections::HashSet;` to `parser.rs` (currently only imports `HashMap`).

5. **Cross-runbook validation scope:** The `validate_runbook_dir()` function in `find.rs` should be called from the daemon/CLI at startup or on runbook reload, not from `parse_runbook_with_format()` (which operates on a single file).

6. **Implicit sequential step flow:** Before implementing dead-end warnings (phase 5), verify how the engine handles `on_done: None`. If it implicitly advances to the next step in the `Vec`, then a missing `on_done` on non-last steps is normal and the dead-end warning should be skipped or rephrased to only flag steps that are truly terminal mid-pipeline (which wouldn't happen with implicit sequential flow).

## Verification Plan

1. **Unit tests** in `crates/runbook/src/parser_tests/references.rs`:
   - Invalid step reference in `on_done` → `ParseError::InvalidFormat`
   - Invalid step reference in `on_fail` → error
   - Invalid step reference in `on_cancel` → error
   - Pipeline-level invalid step reference → error
   - Valid step references → success
   - `run = { agent = "missing" }` → error
   - `run = { pipeline = "missing" }` → error (in step and command)
   - Valid agent/pipeline references → success
   - Duplicate step names in a pipeline → error
   - Unreachable step warning (test via helper function or tracing capture)
   - Dead-end step warning (test via helper function or tracing capture)

2. **Unit tests** in `crates/runbook/src/find_tests.rs`:
   - Two runbook files defining the same command → `FindError::Duplicate`
   - Two runbook files defining the same agent → `FindError::Duplicate`
   - Same name in different entity types → OK

3. **Integration:** `make check` passes (fmt, clippy, tests, build, audit, deny).

4. **Manual verification:** Create a test `.hcl` runbook with intentional errors and confirm parse-time rejection with clear error messages.
