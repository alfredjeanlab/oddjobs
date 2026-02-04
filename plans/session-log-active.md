# Plan: Auto-Resume Escalated Steps on Agent Activity

## Overview

When an agent pipeline step is escalated (status `Waiting`), and the session log subsequently shows the agent is actively working (non-permission `tool_use` or `thinking` blocks), the step should automatically transition back to `Running`. This handles cases where a human manually intervenes in the tmux session, or the agent recovers on its own — the step status should reflect reality without requiring explicit decision resolution.

Currently, `handle_monitor_state` returns `Ok(vec![])` for `MonitorState::Working`, so no state transition occurs even when the pipeline step is in `Waiting` state. The same gap exists for standalone agent runs in `handle_standalone_monitor_state`.

## Project Structure

Files to modify:

```
crates/engine/src/runtime/monitor.rs    # Pipeline: handle Working state for Waiting steps
crates/engine/src/runtime/agent_run.rs  # Standalone: handle Working state for Escalated runs
```

Files for tests:

```
crates/engine/src/runtime/monitor.rs    # Existing test module (monitor_tests.rs)
crates/engine/src/runtime/agent_run.rs  # May need new test functions
```

## Dependencies

No new external dependencies. Uses existing event types (`StepStarted`, `AgentRunStatusChanged`) and effect infrastructure.

## Implementation Phases

### Phase 1: Pipeline steps — auto-resume from Waiting on Working

**File:** `crates/engine/src/runtime/monitor.rs`

In `handle_monitor_state`, replace the early return for `MonitorState::Working` with a check for `Waiting` step status:

```rust
MonitorState::Working => {
    if pipeline.step_status.is_waiting() {
        tracing::info!(
            pipeline_id = %pipeline.id,
            step = %pipeline.step,
            "agent active, auto-resuming from escalation"
        );
        self.logger.append(
            &pipeline.id,
            &pipeline.step,
            "agent active, auto-resuming from escalation",
        );

        let pipeline_id = PipelineId::new(&pipeline.id);
        let effects = vec![
            Effect::Emit {
                event: Event::StepStarted {
                    pipeline_id: pipeline_id.clone(),
                    step: pipeline.step.clone(),
                    agent_id: None,
                    agent_name: None,
                },
            },
        ];
        return Ok(self.executor.execute_all(effects).await?);
    }
    return Ok(vec![]);
}
```

**Verification:** Run `cargo test -p oj-engine` and verify the existing `handle_monitor_state` tests pass. Add a new test that creates a pipeline in `Waiting` state and verifies that `MonitorState::Working` produces a `StepStarted` event.

### Phase 2: Standalone agent runs — auto-resume from Escalated on Working

**File:** `crates/engine/src/runtime/agent_run.rs`

In `handle_standalone_monitor_state`, replace the early return for `MonitorState::Working` with a check for `Escalated` status:

```rust
MonitorState::Working => {
    if agent_run.status == AgentRunStatus::Escalated {
        tracing::info!(
            agent_run_id = %agent_run.id,
            "standalone agent active, auto-resuming from escalation"
        );

        let agent_run_id = AgentRunId::new(&agent_run.id);
        let effects = vec![
            Effect::Emit {
                event: Event::AgentRunStatusChanged {
                    id: agent_run_id,
                    status: AgentRunStatus::Running,
                    reason: Some("agent active".to_string()),
                },
            },
        ];
        return Ok(self.executor.execute_all(effects).await?);
    }
    return Ok(vec![]);
}
```

**Verification:** Run `cargo test -p oj-engine` for standalone agent tests.

### Phase 3: Reset action attempts on auto-resume

When the agent demonstrates it's actively working (auto-resume from escalation), action attempt counters should be reset. This prevents stale attempt counts from carrying over — if the agent becomes idle again later, it's a fresh escalation cycle.

**File:** `crates/engine/src/runtime/monitor.rs`

After emitting `StepStarted`, reset the action attempts on the pipeline:

```rust
// Reset action attempts — agent demonstrated progress
self.lock_state_mut(|state| {
    if let Some(p) = state.pipelines.get_mut(pipeline_id.as_str()) {
        p.reset_action_attempts();
    }
});
```

**File:** `crates/engine/src/runtime/agent_run.rs`

Similarly for standalone agent runs:

```rust
// Reset action attempts
self.lock_state_mut(|state| {
    if let Some(ar) = state.agent_runs.get_mut(agent_run_id.as_str()) {
        ar.action_attempts.clear();
    }
});
```

**Verification:** Add test that verifies attempt counters are reset after auto-resume. Verify that a subsequent idle escalation starts from attempt 1.

### Phase 4: Tests

Add unit tests covering:

1. **Pipeline auto-resume**: Pipeline step in `Waiting` state receives `MonitorState::Working` → emits `StepStarted`, step transitions to `Running`
2. **Pipeline no-op when Running**: Pipeline step in `Running` state receives `MonitorState::Working` → no events emitted (existing behavior preserved)
3. **Standalone auto-resume**: Agent run in `Escalated` status receives `MonitorState::Working` → emits `AgentRunStatusChanged { status: Running }`
4. **Standalone no-op when Running**: Agent run in `Running` status receives `MonitorState::Working` → no events emitted
5. **Attempt reset**: After auto-resume, action attempts are reset to 0

**Verification:** `cargo test -p oj-engine`, then full `make check`.

## Key Implementation Details

### Why this works without watcher changes

The existing session log watcher (`crates/adapters/src/agent/watcher.rs:491-564`) already correctly detects `tool_use` and `thinking` blocks as `AgentState::Working`. It emits `Event::AgentWorking` when the state transitions. Permission prompts are detected separately via CLI hooks (`Notification` hook → `Event::AgentPrompt`), not by the session log parser. Therefore, `AgentWorking` already represents "non-permission tool or thinking block" — no watcher changes needed.

### Event flow

```
Session log: tool_use or thinking block detected
  → Watcher: AgentState::Working → Event::AgentWorking
    → Runtime: handle_agent_state_changed()
      → handle_monitor_state(MonitorState::Working)
        → [NEW] Check pipeline.step_status.is_waiting()
          → Emit StepStarted → step transitions to Running
```

### Race condition: Working → permission prompt

If the watcher detects `tool_use` (emits `AgentWorking`), but then a permission prompt fires immediately after (emits `AgentPrompt`), the step would briefly be `Running`, then `Waiting` again via the prompt handler. This is correct behavior — the step status accurately reflects the agent's state at each point.

### Decision lifecycle

When the step auto-resumes to `Running`, any associated decision remains in state. This is intentional:
- The decision becomes stale but harmless — the pipeline step has moved on
- If the user resolves it, the pipeline handler will still process it correctly
- Decisions are cleaned up when the pipeline reaches terminal state (`PipelineAdvanced` to done/failed/cancelled)

## Verification Plan

1. **Unit tests** (Phase 4): Cover the auto-resume logic for both pipeline and standalone paths
2. **Integration test**: Verify end-to-end that an escalated pipeline step transitions to Running when the session log shows activity
3. **`make check`**: Full CI verification (format, clippy, build, tests, deny)
4. **Manual test**: Start a pipeline, let it escalate on idle, manually type in the tmux session, observe the step status change from `waiting` to `running` in `oj status`
