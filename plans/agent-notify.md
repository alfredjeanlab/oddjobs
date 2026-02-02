# Agent Lifecycle Notifications

## Overview

Agent definitions already support `notify {}` blocks with `on_start`, `on_done`, and `on_fail` templates (parsed in `crates/runbook/src/agent.rs`), but these are never emitted at runtime. This plan wires up `Effect::Notify` emissions at the three agent lifecycle points: spawn, successful completion, and failure. The implementation follows the existing pipeline notification pattern in `runtime/pipeline.rs`.

## Project Structure

Files to modify:

```
crates/engine/src/runtime/monitor.rs   # Emit on_start, on_done, on_fail notifications
crates/engine/src/monitor.rs           # Add notify effect to AdvancePipeline/FailPipeline variants (if needed)
crates/engine/src/monitor_tests.rs     # Unit tests for agent notification emission
docs/interface/DESKTOP.md              # Document agent notify blocks
```

No new files or crates are needed.

## Dependencies

None — all required infrastructure (`Effect::Notify`, `NotifyConfig::render`, template interpolation) already exists.

## Implementation Phases

### Phase 1: Emit `on_start` notification when agent spawns

In `runtime/monitor.rs`, the `spawn_agent()` method (line 94) has access to the `agent_def` (fetched at line 102) and the `pipeline`. After the agent is successfully spawned (after line 155), emit the `on_start` notification if configured.

**Pattern** (mirrors `pipeline_create.rs:176-196`):

```rust
// After executing spawn effects (end of spawn_agent):
if let Some(template) = &agent_def.notify.on_start {
    let mut vars: HashMap<String, String> = pipeline
        .vars
        .iter()
        .map(|(k, v)| (format!("var.{}", k), v.clone()))
        .collect();
    vars.insert("pipeline_id".to_string(), pipeline_id.to_string());
    vars.insert("name".to_string(), pipeline.name.clone());
    vars.insert("agent".to_string(), agent_name.to_string());
    vars.insert("step".to_string(), pipeline.step.clone());

    let message = NotifyConfig::render(template, &vars);
    if let Some(event) = self
        .executor
        .execute(Effect::Notify {
            title: agent_name.to_string(),
            message,
        })
        .await?
    {
        result_events.push(event);
    }
}
```

Note: `spawn_agent` currently returns `Ok(self.executor.execute_all(effects).await?)` directly. This will need to be refactored to capture the result into a `let mut result_events = ...` so we can append the notify effect before returning.

**Verification**: Run `cargo test -p oj-engine` — existing tests should still pass. New test added in Phase 3.

### Phase 2: Emit `on_done` and `on_fail` notifications on agent lifecycle transitions

Agent completion/failure is determined in `handle_monitor_state()` (line 158) and `execute_action_effects()` (line 388). The key insight is that `on_done` and `on_fail` map to specific `ActionEffects` outcomes:

- **`on_done`**: Emit when `ActionEffects::AdvancePipeline` is about to be executed (the agent action resolved to `done` or a gate passed). This happens in `execute_action_effects()` at the `ActionEffects::AdvancePipeline` arm (line 399).
- **`on_fail`**: Emit when `ActionEffects::FailPipeline` is about to be executed (the agent action resolved to `fail`). This happens at line 400.

Additionally, `handle_agent_done()` (line 551) handles explicit `AgentSignalKind::Complete` signals which also advance the pipeline — emit `on_done` there as well.

**Implementation in `execute_action_effects()`**:

The method needs access to the `agent_def` to read its `notify` config. Currently it only receives `pipeline` and `effects`. Add `agent_def: &AgentDef` as a parameter.

```rust
pub(crate) async fn execute_action_effects(
    &self,
    pipeline: &Pipeline,
    agent_def: &oj_runbook::AgentDef,
    effects: ActionEffects,
) -> Result<Vec<Event>, RuntimeError> {
    match effects {
        ActionEffects::AdvancePipeline => {
            // Emit agent on_done notification before advancing
            self.emit_agent_notify(pipeline, agent_def, agent_def.notify.on_done.as_ref()).await?;
            self.advance_pipeline(pipeline).await
        }
        ActionEffects::FailPipeline { error } => {
            // Emit agent on_fail notification before failing
            self.emit_agent_notify(pipeline, agent_def, agent_def.notify.on_fail.as_ref()).await?;
            self.fail_pipeline(pipeline, &error).await
        }
        // ... other arms unchanged
    }
}
```

Update all call sites of `execute_action_effects` to pass `agent_def`:
- `execute_action_with_attempts` (line 310) — already has `agent_def` parameter
- `execute_action_effects` in gate-failed escalation (line 456) — fetch agent_def from runbook (already done at line 453)

**Add `emit_agent_notify` helper** (in `runtime/monitor.rs`):

```rust
async fn emit_agent_notify(
    &self,
    pipeline: &Pipeline,
    agent_def: &oj_runbook::AgentDef,
    message_template: Option<&String>,
) -> Result<(), RuntimeError> {
    if let Some(template) = message_template {
        let mut vars: HashMap<String, String> = pipeline
            .vars
            .iter()
            .map(|(k, v)| (format!("var.{}", k), v.clone()))
            .collect();
        vars.insert("pipeline_id".to_string(), pipeline.id.clone());
        vars.insert("name".to_string(), pipeline.name.clone());
        vars.insert("agent".to_string(), agent_def.name.clone());
        vars.insert("step".to_string(), pipeline.step.clone());
        if let Some(err) = &pipeline.error {
            vars.insert("error".to_string(), err.clone());
        }

        let message = NotifyConfig::render(template, &vars);
        self.executor
            .execute(Effect::Notify {
                title: agent_def.name.clone(),
                message,
            })
            .await?;
    }
    Ok(())
}
```

**For `handle_agent_done` (AgentSignalKind::Complete)**: Fetch the agent_def from the runbook and emit `on_done` before calling `advance_pipeline`:

```rust
AgentSignalKind::Complete => {
    tracing::info!(pipeline_id = %pipeline.id, "agent:signal complete");
    self.logger.append(&pipeline.id, &pipeline.step, "agent:signal complete");

    // Emit agent on_done notification
    if let Ok(runbook) = self.cached_runbook(&pipeline.runbook_hash) {
        if let Ok(agent_def) = crate::monitor::get_agent_def(&runbook, &pipeline) {
            self.emit_agent_notify(&pipeline, agent_def, agent_def.notify.on_done.as_ref()).await?;
        }
    }

    self.advance_pipeline(&pipeline).await
}
```

**Verification**: `cargo test -p oj-engine` — existing tests pass. New tests in Phase 3.

### Phase 3: Unit tests

Add tests in `crates/engine/src/monitor_tests.rs` that verify `Effect::Notify` is produced by `build_action_effects` when agent notify config is set.

Since `build_action_effects` is a pure function that returns `ActionEffects` (not `Effect::Notify` directly), the agent notify emissions happen in the runtime methods. Two testing approaches:

**Approach A — Test `build_action_effects` for existing behavior** (already covered) and add integration-style tests using the `FakeExecutor` pattern from existing runtime tests.

**Approach B — Unit-test the `emit_agent_notify` helper directly** if it can be extracted as a pure function that returns `Option<Effect>` instead of executing. This is preferred for testability:

```rust
/// Build an agent notification effect if a message template is configured.
pub(crate) fn build_agent_notify_effect(
    pipeline: &Pipeline,
    agent_def: &AgentDef,
    message_template: Option<&String>,
) -> Option<Effect> {
    let template = message_template?;
    let mut vars: HashMap<String, String> = pipeline
        .vars
        .iter()
        .map(|(k, v)| (format!("var.{}", k), v.clone()))
        .collect();
    vars.insert("pipeline_id".to_string(), pipeline.id.clone());
    vars.insert("name".to_string(), pipeline.name.clone());
    vars.insert("agent".to_string(), agent_def.name.clone());
    vars.insert("step".to_string(), pipeline.step.clone());
    if let Some(err) = &pipeline.error {
        vars.insert("error".to_string(), err.clone());
    }

    let message = NotifyConfig::render(template, &vars);
    Some(Effect::Notify {
        title: agent_def.name.clone(),
        message,
    })
}
```

Then the runtime `emit_agent_notify` becomes:

```rust
async fn emit_agent_notify(&self, pipeline: &Pipeline, agent_def: &AgentDef, template: Option<&String>) -> Result<(), RuntimeError> {
    if let Some(effect) = build_agent_notify_effect(pipeline, agent_def, template) {
        self.executor.execute(effect).await?;
    }
    Ok(())
}
```

**Tests** (in `monitor_tests.rs`):

```rust
#[test]
fn agent_on_start_notify_renders_template() {
    let pipeline = test_pipeline();
    let mut agent = test_agent_def();
    agent.notify.on_start = Some("Agent ${agent} started for ${name}".to_string());

    let effect = build_agent_notify_effect(&pipeline, &agent, agent.notify.on_start.as_ref());
    assert!(effect.is_some());
    match effect.unwrap() {
        Effect::Notify { title, message } => {
            assert_eq!(title, "worker");
            assert_eq!(message, "Agent worker started for test-feature");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn agent_on_done_notify_renders_template() {
    let pipeline = test_pipeline();
    let mut agent = test_agent_def();
    agent.notify.on_done = Some("Agent ${agent} completed".to_string());

    let effect = build_agent_notify_effect(&pipeline, &agent, agent.notify.on_done.as_ref());
    match effect.unwrap() {
        Effect::Notify { title, message } => {
            assert_eq!(title, "worker");
            assert_eq!(message, "Agent worker completed");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn agent_on_fail_notify_includes_error() {
    let mut pipeline = test_pipeline();
    pipeline.error = Some("task failed".to_string());
    let mut agent = test_agent_def();
    agent.notify.on_fail = Some("Agent ${agent} failed: ${error}".to_string());

    let effect = build_agent_notify_effect(&pipeline, &agent, agent.notify.on_fail.as_ref());
    match effect.unwrap() {
        Effect::Notify { title, message } => {
            assert_eq!(title, "worker");
            assert_eq!(message, "Agent worker failed: task failed");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn agent_notify_none_when_no_template() {
    let pipeline = test_pipeline();
    let agent = test_agent_def(); // no notify config
    let effect = build_agent_notify_effect(&pipeline, &agent, None);
    assert!(effect.is_none());
}

#[test]
fn agent_notify_interpolates_pipeline_vars() {
    let mut pipeline = test_pipeline();
    pipeline.vars.insert("env".to_string(), "prod".to_string());
    let mut agent = test_agent_def();
    agent.notify.on_start = Some("Deploying ${var.env}".to_string());

    let effect = build_agent_notify_effect(&pipeline, &agent, agent.notify.on_start.as_ref());
    match effect.unwrap() {
        Effect::Notify { message, .. } => {
            assert_eq!(message, "Deploying prod");
        }
        _ => panic!("expected Notify effect"),
    }
}
```

**Verification**: `cargo test -p oj-engine -- monitor_tests`

### Phase 4: Update documentation

Update `docs/interface/DESKTOP.md` to add agent notification rows to the existing table and a new section documenting agent notify blocks.

Add to the notification table:

| Event | Title | Message |
|-------|-------|---------|
| Agent `on_start` | Agent name | Rendered `on_start` template |
| Agent `on_done` | Agent name | Rendered `on_done` template |
| Agent `on_fail` | Agent name | Rendered `on_fail` template |

Add a new subsection after the table:

```markdown
### Agent Notifications

Agents support the same `notify {}` block as pipelines to emit desktop notifications on lifecycle events:

    agent "worker" {
      run    = "claude"
      prompt = "Implement the feature."

      notify {
        on_start = "Agent ${agent} started on ${name}"
        on_done  = "Agent ${agent} completed"
        on_fail  = "Agent ${agent} failed: ${error}"
      }
    }

Available template variables:

| Variable | Description |
|----------|-------------|
| `${var.*}` | Pipeline variables (e.g. `${var.env}`) |
| `${pipeline_id}` | Pipeline ID |
| `${name}` | Pipeline name |
| `${agent}` | Agent name |
| `${step}` | Current step name |
| `${error}` | Error message (available in `on_fail`) |
```

**Verification**: Review the rendered markdown.

## Key Implementation Details

1. **Notification title uses agent name, not pipeline name.** Pipeline notifications use `pipeline.name` as the title. Agent notifications use `agent_def.name` to distinguish agent-level notifications from pipeline-level ones.

2. **Pure function for testability.** The core logic is a pure `build_agent_notify_effect()` function in `crates/engine/src/monitor.rs` that returns `Option<Effect>`. The runtime method `emit_agent_notify()` is a thin wrapper that executes the effect. This allows unit testing without needing a full runtime setup.

3. **`execute_action_effects` signature change.** Adding `agent_def: &AgentDef` to `execute_action_effects` threads the notify config through to the AdvancePipeline and FailPipeline arms. All existing call sites already have the agent_def available.

4. **`handle_agent_done` fetches agent_def from runbook.** For `AgentSignalKind::Complete`, the agent_def is looked up from the cached runbook (same pattern used throughout `monitor.rs` e.g. `recover_agent` at line 53).

5. **`on_fail` error variable.** The `${error}` variable in `on_fail` templates comes from `pipeline.error`. When `FailPipeline` is triggered, the error string is available. For the `on_fail` notification to include the error, we set `pipeline.error` in the vars map. Note: at the point `execute_action_effects` handles `FailPipeline { error }`, the error is in the `error` field of the enum variant — this should be added to vars before rendering.

## Verification Plan

1. **`make check`** — full CI verification (fmt, clippy, tests, build, audit, deny)
2. **Unit tests** — `cargo test -p oj-engine -- monitor_tests` verifies:
   - `on_start` template renders with agent name and pipeline vars
   - `on_done` template renders on completion
   - `on_fail` template renders with error message
   - No notification emitted when template is `None`
   - Pipeline vars interpolated correctly with `${var.*}` prefix
3. **Manual smoke test** — Create a runbook with agent notify block, run a pipeline, verify desktop notifications appear at each lifecycle point
