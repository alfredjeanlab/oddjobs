# Cron Concurrency

## Overview

Add a `concurrency` setting to cron definitions that limits how many active pipelines a cron can have running simultaneously. Default to `concurrency = 1` so crons are singleton by default — if a previous pipeline from this cron is still active, the tick is skipped. This mirrors how `max_concurrency` already works for agent crons, extending the same pattern to pipeline crons.

The implementation requires:
1. A new `concurrency` field on `CronDef` (runbook schema)
2. A new `cron_name` field on `Pipeline` (to track provenance)
3. A `count_active_cron_pipelines` helper on `Runtime`
4. A concurrency gate in `handle_cron_timer_fired` before spawning pipeline targets

## Project Structure

Key files to modify:

```
crates/
├── runbook/src/cron.rs              # Add concurrency field to CronDef
├── core/src/pipeline.rs             # Add cron_name field to Pipeline + PipelineConfig
├── core/src/event.rs                # Add cron_name field to PipelineCreated event
├── storage/src/state.rs             # Wire cron_name through apply_event for PipelineCreated
├── engine/src/runtime/mod.rs        # Add count_active_cron_pipelines helper
├── engine/src/runtime/handlers/
│   ├── cron.rs                      # Add concurrency check + pass cron_name, update CronState
│   └── pipeline_create.rs           # Thread cron_name through CreatePipelineParams
└── engine/src/runtime_tests/cron.rs # New tests for pipeline concurrency
```

## Dependencies

No new external dependencies required. All changes use existing types and patterns.

## Implementation Phases

### Phase 1: Schema — Add `concurrency` to `CronDef`

Add the `concurrency` field to the runbook cron definition in `crates/runbook/src/cron.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronDef {
    #[serde(skip)]
    pub name: String,
    pub interval: String,
    pub run: RunDirective,
    /// Maximum number of active pipelines this cron can have running
    /// simultaneously. Defaults to 1 (singleton). `None` means use default.
    #[serde(default)]
    pub concurrency: Option<u32>,
}
```

Default to `None` which the engine interprets as `1`. This keeps existing runbooks backward-compatible (no field = singleton behavior).

**Verify:** `cargo build -p oj-runbook` compiles. Existing HCL/TOML runbooks parse without the field.

### Phase 2: Pipeline provenance — Add `cron_name` to `Pipeline`

Pipelines need to track which cron spawned them so the engine can count active pipelines per cron.

**2a. `PipelineConfig` and `Pipeline` (`crates/core/src/pipeline.rs`)**

Add `cron_name: Option<String>` to both `PipelineConfig` and `Pipeline`:

```rust
pub struct PipelineConfig {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub vars: HashMap<String, String>,
    pub runbook_hash: String,
    pub cwd: PathBuf,
    pub initial_step: String,
    pub namespace: String,
    pub cron_name: Option<String>,  // NEW
}
```

In `Pipeline`:
```rust
/// Name of the cron that spawned this pipeline, if any.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub cron_name: Option<String>,
```

Initialize from `config.cron_name` in `Pipeline::new_with_epoch_ms`.

**2b. `PipelineCreated` event (`crates/core/src/event.rs`)**

Add `cron_name` to the event so it persists through WAL replay:

```rust
PipelineCreated {
    id: PipelineId,
    kind: String,
    name: String,
    runbook_hash: String,
    cwd: PathBuf,
    vars: HashMap<String, String>,
    initial_step: String,
    created_at_epoch_ms: u64,
    namespace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cron_name: Option<String>,  // NEW
}
```

**2c. `MaterializedState::apply_event` (`crates/storage/src/state.rs`)**

Thread `cron_name` through the `PipelineCreated` match arm into `PipelineConfig`:

```rust
Event::PipelineCreated {
    id, kind, name, runbook_hash, cwd, vars,
    initial_step, created_at_epoch_ms, namespace, cron_name,
} => {
    let config = PipelineConfig {
        id: id.to_string(),
        name: name.clone(),
        kind: kind.clone(),
        vars: vars.clone(),
        runbook_hash: runbook_hash.clone(),
        cwd: cwd.clone(),
        initial_step: initial_step.clone(),
        namespace: namespace.clone(),
        cron_name: cron_name.clone(),  // NEW
    };
    let pipeline = Pipeline::new_with_epoch_ms(config, *created_at_epoch_ms);
    self.pipelines.insert(id.to_string(), pipeline);
}
```

**2d. `CreatePipelineParams` + `create_and_start_pipeline` (`crates/engine/src/runtime/handlers/pipeline_create.rs`)**

Add `cron_name: Option<String>` to `CreatePipelineParams`. Pass it through to the `PipelineCreated` event emission:

```rust
pub(crate) struct CreatePipelineParams {
    pub pipeline_id: PipelineId,
    pub pipeline_name: String,
    pub pipeline_kind: String,
    pub vars: HashMap<String, String>,
    pub runbook_hash: String,
    pub runbook_json: Option<serde_json::Value>,
    pub runbook: Runbook,
    pub namespace: String,
    pub cron_name: Option<String>,  // NEW
}
```

In `create_and_start_pipeline`, add `cron_name: params.cron_name` (or destructured equivalent) to the `Event::PipelineCreated { .. }` emission.

Update all existing call sites of `CreatePipelineParams` to pass `cron_name: None` (non-cron callers). There are call sites in:
- `crates/engine/src/runtime/handlers/cron.rs` (will pass `Some(cron_name)`)
- `crates/engine/src/runtime/handlers/command.rs` (pass `None`)
- `crates/engine/src/runtime/handlers/worker.rs` (pass `None`)

**Verify:** `cargo build --all` compiles. `cargo test --all` passes (existing tests don't break).

### Phase 3: Concurrency check in cron handler

**3a. Store `concurrency` in `CronState` (`crates/engine/src/runtime/handlers/cron.rs`)**

Add the concurrency setting to in-memory cron state so it's available at timer-fire time:

```rust
pub(crate) struct CronState {
    pub project_root: PathBuf,
    pub runbook_hash: String,
    pub interval: String,
    pub run_target: CronRunTarget,
    pub status: CronStatus,
    pub namespace: String,
    pub concurrency: u32,  // NEW — default 1
}
```

Populate from the runbook's `CronDef` during `handle_cron_started`. Use `cron_def.concurrency.unwrap_or(1)`.

Note: The cron handler already refreshes the runbook from disk each tick (`refresh_cron_runbook`). After a refresh, also re-read the `concurrency` value from the updated `CronDef` so hot-reloading works. Do this right after the runbook hash is re-read (around line 343-349), alongside any other CronState updates that reference the new runbook.

**3b. Add `count_active_cron_pipelines` helper (`crates/engine/src/runtime/mod.rs`)**

Mirror the existing `count_running_agents` pattern:

```rust
/// Count currently active (non-terminal) pipelines spawned by a given cron.
pub(crate) fn count_active_cron_pipelines(&self, cron_name: &str, namespace: &str) -> usize {
    self.lock_state(|state| {
        state
            .pipelines
            .values()
            .filter(|p| {
                p.cron_name.as_deref() == Some(cron_name)
                    && p.namespace == namespace
                    && !p.is_terminal()
            })
            .count()
    })
}
```

**3c. Add concurrency gate in `handle_cron_timer_fired` (`crates/engine/src/runtime/handlers/cron.rs`)**

In the `CronRunTarget::Pipeline` arm, before generating the pipeline ID, add the concurrency check. This mirrors the agent concurrency check (lines 415-444):

```rust
CronRunTarget::Pipeline(pipeline_name) => {
    // Check concurrency before spawning
    let concurrency = cron_state.concurrency; // read from CronState
    let active = self.count_active_cron_pipelines(cron_name, &namespace);
    if active >= concurrency as usize {
        append_cron_log(
            self.logger.log_dir(),
            cron_name,
            &namespace,
            &format!(
                "skip: pipeline '{}' at max concurrency ({}/{})",
                pipeline_name, active, concurrency
            ),
        );
        // Reschedule timer but don't spawn
        let duration = crate::monitor::parse_duration(&interval).map_err(|e| {
            RuntimeError::InvalidFormat(format!(
                "invalid cron interval '{}': {}",
                interval, e
            ))
        })?;
        let timer_id = TimerId::cron(cron_name, &namespace);
        self.executor
            .execute(Effect::SetTimer {
                id: timer_id,
                duration,
            })
            .await?;
        return Ok(result_events);
    }

    // ... existing pipeline spawn logic ...
}
```

**3d. Pass `cron_name` when spawning from cron**

In `handle_cron_timer_fired`, pass `cron_name: Some(cron_name.to_string())` in the `CreatePipelineParams`.

In `handle_cron_once`, similarly pass the cron name through.

**Verify:** `cargo build --all` compiles. Unit tests pass.

### Phase 4: Tests

Add tests to `crates/engine/src/runtime_tests/cron.rs` mirroring the existing agent concurrency tests (tests 13-15):

**Test: `cron_pipeline_concurrency_skip`** — Mirrors `cron_agent_concurrency_skip`:
1. Create a runbook with `concurrency = 1` on a cron that targets a pipeline
2. Inject an active (non-terminal) pipeline with `cron_name = Some("the_cron")`
3. Start the cron, fire the timer
4. Assert: no `PipelineCreated` event is emitted
5. Assert: no `CronFired` event is emitted
6. Assert: timer is rescheduled

**Test: `cron_pipeline_concurrency_respawns_after_complete`** — Mirrors `cron_agent_concurrency_respawns_after_complete`:
1. Create a runbook with `concurrency = 1` on a cron that targets a pipeline
2. Inject a completed (terminal) pipeline with `cron_name = Some("the_cron")`
3. Start the cron, fire the timer
4. Assert: `PipelineCreated` is emitted
5. Assert: `CronFired` is emitted

**Test: `cron_pipeline_concurrency_default_singleton`**:
1. Create a runbook with NO `concurrency` field on a pipeline cron
2. Inject an active pipeline with matching `cron_name`
3. Fire the timer
4. Assert: spawn is skipped (default concurrency=1 makes it singleton)

**Test: `cron_pipeline_concurrency_allows_multiple`**:
1. Create a runbook with `concurrency = 2`
2. Inject one active pipeline with matching `cron_name`
3. Fire the timer
4. Assert: `PipelineCreated` IS emitted (1 < 2, room for another)

Add a test runbook constant like:
```rust
const CRON_PIPELINE_CONC_RUNBOOK: &str = r#"
[cron.deployer]
interval = "10m"
concurrency = 1
run = { pipeline = "deploy" }

[pipeline.deploy]
[[pipeline.deploy.step]]
name = "run"
run = "echo deploying"
"#;
```

**Verify:** `cargo test --all` passes, including the new tests.

### Phase 5: Agent cron concurrency unification (optional cleanup)

The agent cron path reads `max_concurrency` from `AgentDef`, not from `CronDef`. Now that crons have their own `concurrency` field, the pipeline path uses `CronDef.concurrency`. For agents, the existing behavior (reading from `AgentDef.max_concurrency`) should continue to work as-is. No changes needed here for correctness — this is noted for future consideration only.

If desired, a follow-up could make the cron-level `concurrency` field also apply to agent crons (taking precedence over `AgentDef.max_concurrency` when set). This is out of scope for this plan.

## Key Implementation Details

### Default behavior
`concurrency` defaults to `1` when not specified. This makes all pipeline crons singleton by default — the most common and safest behavior. Users can set `concurrency = 0` or omit-and-override to allow unlimited concurrent pipelines if needed, though `0` meaning "unlimited" should be documented.

### Backward compatibility
- The `cron_name` field on `Pipeline` and `PipelineCreated` uses `#[serde(default)]` so old WAL entries deserialize correctly (as `None`).
- Old pipelines without `cron_name` won't be counted against any cron's concurrency, which is correct — they predate the feature.
- `CronDef.concurrency` uses `#[serde(default)]` so existing runbooks parse without the field.

### Terminal state check
`Pipeline::is_terminal()` already checks for `step == "done" || step == "failed" || step == "cancelled"`. The `count_active_cron_pipelines` helper reuses this, matching the pattern from `count_running_agents` which uses `AgentRunStatus::is_terminal()`.

### Hot reload
When `refresh_cron_runbook` detects a runbook change, the concurrency value in `CronState` should be updated from the new `CronDef`. This allows operators to change concurrency limits without restarting the cron.

## Verification Plan

1. **Unit tests:** Run `cargo test -p oj-engine` — new tests cover skip, respawn, default-singleton, and multi-concurrency scenarios
2. **Full suite:** Run `make check` — ensures clippy, fmt, all tests, and deny checks pass
3. **WAL compat:** Verify old snapshots deserialize correctly (serde defaults handle missing fields)
4. **Manual test:** Create a runbook with a slow pipeline (e.g., `sleep 60`) on a short cron interval (e.g., `1m`), observe that only one instance runs at a time with default settings
