# Escalation Decisions — Phase 3: Connect Escalation Paths to Decision System

## Overview

Wire escalation paths (`Action::Escalate` from `on_idle`, `on_dead`, `on_error`, and gate failures) to emit `DecisionCreated` events instead of bare `StepWaiting` events. Each decision includes trigger-specific context and system-generated options (retry, nudge, cancel). Desktop notifications now trigger on decision creation, replacing the current escalation notify pattern.

## Project Structure

Modified files:
```
crates/engine/src/monitor.rs                    # Emit DecisionCreated in build_action_effects
crates/engine/src/runtime/monitor.rs            # Update gate failure path to emit decision
crates/engine/src/runtime/agent_run.rs          # Standalone agent escalation decisions (optional)
crates/core/src/decision.rs                     # Add helper for system-generated options
crates/daemon/src/listener/decisions.rs         # Handle decision resolution → pipeline action
```

New files:
```
crates/engine/src/decision_builder.rs           # Builder for escalation decisions
crates/engine/src/decision_builder_tests.rs     # Unit tests
```

Test files to update:
```
crates/engine/src/runtime_tests/mod.rs          # Update escalation tests
crates/storage/src/state_tests/mod.rs           # Test DecisionCreated from escalation
```

## Dependencies

No new external crate dependencies. Uses existing:
- `uuid` for decision ID generation
- `std::time` for `created_at_ms` timestamps

Internal dependency: Phase 1 (decisions-phase1.md) must be complete — `DecisionCreated`, `DecisionResolved` events and the decision data model must exist.

## Implementation Phases

### Phase 1: Decision Builder Module

**Goal:** Create a reusable builder for constructing escalation decisions with system-generated options.

**1a. Create `crates/engine/src/decision_builder.rs`:**

```rust
use oj_core::{DecisionOption, DecisionSource, Event, PipelineId};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Trigger that caused the escalation.
#[derive(Debug, Clone)]
pub enum EscalationTrigger {
    /// Agent was idle for too long (on_idle)
    Idle,
    /// Agent process died unexpectedly (on_dead)
    Dead { exit_code: Option<i32> },
    /// Agent encountered an API/runtime error (on_error)
    Error { error_type: String, message: String },
    /// Gate command failed (gate action)
    GateFailed {
        command: String,
        exit_code: i32,
        stderr: String,
    },
    /// Agent showed a permission prompt we couldn't handle (on_prompt)
    Prompt { prompt_type: String },
}

impl EscalationTrigger {
    pub fn to_source(&self) -> DecisionSource {
        match self {
            EscalationTrigger::Idle => DecisionSource::Idle,
            EscalationTrigger::Dead { .. } => DecisionSource::Error,
            EscalationTrigger::Error { .. } => DecisionSource::Error,
            EscalationTrigger::GateFailed { .. } => DecisionSource::Gate,
            EscalationTrigger::Prompt { .. } => DecisionSource::Approval,
        }
    }
}

/// Build a DecisionCreated event for an escalation.
pub struct EscalationDecisionBuilder {
    pipeline_id: PipelineId,
    pipeline_name: String,
    agent_id: Option<String>,
    trigger: EscalationTrigger,
    agent_log_tail: Option<String>,
    namespace: String,
}

impl EscalationDecisionBuilder {
    pub fn new(
        pipeline_id: PipelineId,
        pipeline_name: String,
        trigger: EscalationTrigger,
    ) -> Self {
        Self {
            pipeline_id,
            pipeline_name,
            agent_id: None,
            trigger,
            agent_log_tail: None,
            namespace: String::new(),
        }
    }

    pub fn agent_id(mut self, id: impl Into<String>) -> Self {
        self.agent_id = Some(id.into());
        self
    }

    pub fn agent_log_tail(mut self, tail: impl Into<String>) -> Self {
        self.agent_log_tail = Some(tail.into());
        self
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = ns.into();
        self
    }

    /// Build the DecisionCreated event and generated decision ID.
    pub fn build(self) -> (String, Event) {
        let decision_id = Uuid::new_v4().to_string();
        let context = self.build_context();
        let options = self.build_options();
        let created_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = Event::DecisionCreated {
            id: decision_id.clone(),
            pipeline_id: self.pipeline_id,
            agent_id: self.agent_id,
            source: self.trigger.to_source(),
            context,
            options,
            created_at_ms,
            namespace: self.namespace,
        };

        (decision_id, event)
    }

    fn build_context(&self) -> String {
        let mut parts = Vec::new();

        // Trigger-specific header
        match &self.trigger {
            EscalationTrigger::Idle => {
                parts.push(format!(
                    "Agent in pipeline \"{}\" is idle and waiting for input.",
                    self.pipeline_name
                ));
            }
            EscalationTrigger::Dead { exit_code } => {
                let code_str = exit_code
                    .map(|c| format!(" (exit code {})", c))
                    .unwrap_or_default();
                parts.push(format!(
                    "Agent in pipeline \"{}\" exited unexpectedly{}.",
                    self.pipeline_name, code_str
                ));
            }
            EscalationTrigger::Error { error_type, message } => {
                parts.push(format!(
                    "Agent in pipeline \"{}\" encountered an error: {} — {}",
                    self.pipeline_name, error_type, message
                ));
            }
            EscalationTrigger::GateFailed { command, exit_code, stderr } => {
                parts.push(format!(
                    "Gate command failed in pipeline \"{}\".",
                    self.pipeline_name
                ));
                parts.push(format!("Command: {}", command));
                parts.push(format!("Exit code: {}", exit_code));
                if !stderr.is_empty() {
                    parts.push(format!("stderr:\n{}", stderr));
                }
            }
            EscalationTrigger::Prompt { prompt_type } => {
                parts.push(format!(
                    "Agent in pipeline \"{}\" is showing a {} prompt.",
                    self.pipeline_name, prompt_type
                ));
            }
        }

        // Agent log tail if available
        if let Some(tail) = &self.agent_log_tail {
            if !tail.is_empty() {
                parts.push(format!("\nRecent agent output:\n{}", tail));
            }
        }

        parts.join("\n")
    }

    fn build_options(&self) -> Vec<DecisionOption> {
        match &self.trigger {
            EscalationTrigger::Idle => vec![
                DecisionOption {
                    label: "Nudge".to_string(),
                    description: Some("Send a message prompting the agent to continue".to_string()),
                    recommended: true,
                },
                DecisionOption {
                    label: "Done".to_string(),
                    description: Some("Mark as complete and advance the pipeline".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel the pipeline".to_string()),
                    recommended: false,
                },
            ],
            EscalationTrigger::Dead { .. } | EscalationTrigger::Error { .. } => vec![
                DecisionOption {
                    label: "Retry".to_string(),
                    description: Some("Restart the agent with --resume to continue".to_string()),
                    recommended: true,
                },
                DecisionOption {
                    label: "Skip".to_string(),
                    description: Some("Skip this step and advance the pipeline".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel the pipeline".to_string()),
                    recommended: false,
                },
            ],
            EscalationTrigger::GateFailed { .. } => vec![
                DecisionOption {
                    label: "Retry".to_string(),
                    description: Some("Re-run the gate command".to_string()),
                    recommended: true,
                },
                DecisionOption {
                    label: "Skip".to_string(),
                    description: Some("Skip the gate and advance the pipeline".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel the pipeline".to_string()),
                    recommended: false,
                },
            ],
            EscalationTrigger::Prompt { .. } => vec![
                DecisionOption {
                    label: "Approve".to_string(),
                    description: Some("Approve the pending action".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Deny".to_string(),
                    description: Some("Deny the pending action".to_string()),
                    recommended: false,
                },
                DecisionOption {
                    label: "Cancel".to_string(),
                    description: Some("Cancel the pipeline".to_string()),
                    recommended: false,
                },
            ],
        }
    }
}
```

**1b. Update `crates/engine/src/lib.rs`:**

```rust
mod decision_builder;
pub use decision_builder::{EscalationDecisionBuilder, EscalationTrigger};
```

### Phase 2: Update Pipeline Escalation Effects

**Goal:** Replace `StepWaiting` emission with `DecisionCreated` in `build_action_effects()`.

**2a. Update `crates/engine/src/monitor.rs` — `AgentAction::Escalate` arm:**

Current code (lines 201-231):
```rust
AgentAction::Escalate => {
    let effects = vec![
        Effect::Notify { ... },
        Effect::Emit { event: Event::StepWaiting { ... } },
        Effect::CancelTimer { ... },
    ];
    Ok(ActionEffects::Escalate { effects })
}
```

New code:
```rust
AgentAction::Escalate => {
    tracing::warn!(
        pipeline_id = %pipeline.id,
        trigger = trigger,
        message = ?message,
        "escalating to human — creating decision"
    );

    // Determine escalation trigger type from the trigger string
    let escalation_trigger = match trigger {
        "idle" | "on_idle" => EscalationTrigger::Idle,
        "dead" | "on_dead" | "exited" => EscalationTrigger::Dead { exit_code: None },
        "error" | "on_error" => EscalationTrigger::Error {
            error_type: "unknown".to_string(),
            message: message.unwrap_or("").to_string(),
        },
        "prompt" | "on_prompt" => EscalationTrigger::Prompt {
            prompt_type: "permission".to_string(),
        },
        _ => EscalationTrigger::Idle, // fallback
    };

    let (decision_id, decision_event) = EscalationDecisionBuilder::new(
        pipeline_id.clone(),
        pipeline.name.clone(),
        escalation_trigger,
    )
    .agent_id(pipeline.session_id.clone().unwrap_or_default())
    .namespace(pipeline.namespace.clone())
    .build();

    let effects = vec![
        // Emit DecisionCreated (this also sets pipeline to Waiting)
        Effect::Emit { event: decision_event },
        // Desktop notification on decision created
        Effect::Notify {
            title: format!("Decision needed: {}", pipeline.name),
            message: format!("Pipeline requires attention ({})", trigger),
        },
        // Cancel exit-deferred timer (agent may still be alive)
        Effect::CancelTimer {
            id: TimerId::exit_deferred(&pipeline_id),
        },
    ];

    Ok(ActionEffects::Escalate { effects })
}
```

**2b. Extend function signature to accept richer trigger context:**

The `build_action_effects` function currently takes `trigger: &str`. To pass structured trigger info (exit codes, error messages), add an optional `EscalationContext` parameter:

```rust
/// Additional context for escalation triggers.
#[derive(Debug, Clone, Default)]
pub struct EscalationContext {
    pub exit_code: Option<i32>,
    pub error_type: Option<String>,
    pub error_message: Option<String>,
    pub agent_log_tail: Option<String>,
}

pub fn build_action_effects(
    pipeline: &Pipeline,
    agent_def: &AgentDef,
    action_config: &ActionConfig,
    trigger: &str,
    vars: &HashMap<String, String>,
    escalation_ctx: Option<&EscalationContext>,  // NEW
) -> Result<ActionEffects, RuntimeError>
```

Call sites need updating to pass `None` or the context where available.

### Phase 3: Update Gate Failure Escalation

**Goal:** Gate failures emit `DecisionCreated` with command, exit code, and stderr in context.

**3a. Update `crates/engine/src/runtime/monitor.rs` gate failure path (lines 529-576):**

Current code captures `gate_error` string and injects it into `StepWaiting`. Replace with:

```rust
Err(gate_error) => {
    self.logger.append(
        &pipeline.id,
        &pipeline.step,
        &format!("gate failed: {}", gate_error),
    );

    // Parse gate error for structured context
    // gate_error format: "exit code {code}: {stderr}" or similar
    let (exit_code, stderr) = parse_gate_error(&gate_error);

    let (decision_id, decision_event) = EscalationDecisionBuilder::new(
        PipelineId::new(&pipeline.id),
        pipeline.name.clone(),
        EscalationTrigger::GateFailed {
            command: command.clone(),
            exit_code,
            stderr,
        },
    )
    .agent_id(pipeline.session_id.clone().unwrap_or_default())
    .namespace(pipeline.namespace.clone())
    .build();

    let effects = vec![
        Effect::Emit { event: decision_event },
        Effect::Notify {
            title: format!("Gate failed: {}", pipeline.name),
            message: format!("Command '{}' failed", command),
        },
        Effect::CancelTimer {
            id: TimerId::exit_deferred(&PipelineId::new(&pipeline.id)),
        },
    ];

    Ok(self.executor.execute_all(effects).await?)
}
```

**3b. Add helper to parse gate error:**

```rust
/// Parse a gate error string into exit code and stderr.
fn parse_gate_error(error: &str) -> (i32, String) {
    // Expected format from run_gate_command: "exit code {N}: {stderr}"
    if let Some(rest) = error.strip_prefix("exit code ") {
        if let Some((code_str, stderr)) = rest.split_once(':') {
            if let Ok(code) = code_str.trim().parse::<i32>() {
                return (code, stderr.trim().to_string());
            }
        }
    }
    // Fallback: unknown exit code, full string as stderr
    (1, error.to_string())
}
```

### Phase 4: Update Error Escalation with Agent Log Tail

**Goal:** For `on_dead`, `on_idle`, and `on_error` triggers, include agent log tail in decision context.

**4a. Add log tail fetcher to monitor runtime:**

```rust
impl PipelineMonitorRuntime {
    /// Fetch the last N lines from the agent's session log.
    async fn fetch_agent_log_tail(
        &self,
        pipeline: &Pipeline,
        lines: usize,
    ) -> Option<String> {
        let session_id = pipeline.session_id.as_ref()?;
        // Use the log reader to get recent output
        self.logger.tail(&pipeline.id, &pipeline.step, lines)
    }
}
```

**4b. Pass log tail through escalation context:**

When calling `build_action_effects` from the monitor runtime for idle/dead/error triggers, fetch the log tail first and include it in `EscalationContext`:

```rust
// In handle_agent_state_change or similar:
let log_tail = self.fetch_agent_log_tail(pipeline, 50).await;
let ctx = EscalationContext {
    exit_code: exit_code_from_state,
    agent_log_tail: log_tail,
    ..Default::default()
};
let effects = build_action_effects(
    pipeline,
    agent_def,
    &on_idle_config,
    "idle",
    &pipeline.vars,
    Some(&ctx),
)?;
```

### Phase 5: Decision Resolution → Pipeline Action

**Goal:** When a decision is resolved, execute the corresponding action (nudge, retry, cancel).

**5a. Extend `handle_decision_resolve` in `crates/daemon/src/listener/decisions.rs`:**

After emitting `DecisionResolved` and `PipelineResume`, dispatch the action based on the chosen option:

```rust
// After existing resolution logic...

// Map chosen option to pipeline action
if let Some(choice) = chosen {
    let action_event = match (decision.source.as_str(), choice) {
        // Idle decisions: 1=Nudge, 2=Done, 3=Cancel
        ("idle", 1) => Some(Event::PipelineResume {
            id: PipelineId::new(&pipeline_id),
            message: message.clone(),
            vars: HashMap::new(),
        }),
        ("idle", 2) => Some(Event::StepCompleted {
            pipeline_id: PipelineId::new(&pipeline_id),
            step: String::new(), // filled from pipeline state
            outcome: StepOutcome::Success,
        }),
        ("idle", 3) | (_, 3) => Some(Event::PipelineCancelled {
            id: PipelineId::new(&pipeline_id),
            reason: message.unwrap_or_else(|| "cancelled by decision".to_string()),
        }),

        // Error/Dead/Gate decisions: 1=Retry, 2=Skip, 3=Cancel
        ("error" | "gate", 1) => Some(Event::PipelineResume {
            id: PipelineId::new(&pipeline_id),
            message: Some("retrying after decision".to_string()),
            vars: HashMap::new(),
        }),
        ("error" | "gate", 2) => Some(Event::StepCompleted {
            pipeline_id: PipelineId::new(&pipeline_id),
            step: String::new(),
            outcome: StepOutcome::Skipped,
        }),

        _ => None,
    };

    if let Some(event) = action_event {
        event_bus.send(event).map_err(|_| ConnectionError::WalError)?;
    }
}
```

**5b. Consider adding a freeform message nudge:**

When `message` is provided without a `choice` (freeform response), send it to the agent:

```rust
if chosen.is_none() && message.is_some() {
    // Freeform message — send to agent session
    let nudge_event = Event::PipelineResume {
        id: PipelineId::new(&pipeline_id),
        message,
        vars: HashMap::new(),
    };
    event_bus.send(nudge_event).map_err(|_| ConnectionError::WalError)?;
}
```

### Phase 6: Tests & Verification

**Goal:** Comprehensive tests for new escalation decision flow.

**6a. Unit tests for decision builder (`crates/engine/src/decision_builder_tests.rs`):**

```rust
#[test]
fn test_idle_trigger_builds_correct_options() {
    let (id, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
        EscalationTrigger::Idle,
    ).build();

    match event {
        Event::DecisionCreated { options, source, .. } => {
            assert_eq!(source, DecisionSource::Idle);
            assert_eq!(options.len(), 3);
            assert_eq!(options[0].label, "Nudge");
            assert!(options[0].recommended);
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_gate_failure_includes_command_and_stderr() {
    let (_, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
        EscalationTrigger::GateFailed {
            command: "./check.sh".to_string(),
            exit_code: 1,
            stderr: "validation failed".to_string(),
        },
    ).build();

    match event {
        Event::DecisionCreated { context, source, .. } => {
            assert_eq!(source, DecisionSource::Gate);
            assert!(context.contains("./check.sh"));
            assert!(context.contains("validation failed"));
            assert!(context.contains("Exit code: 1"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}
```

**6b. Integration tests for escalation flow:**

- Trigger `on_idle` escalation → verify `DecisionCreated` event emitted
- Trigger gate failure → verify decision includes command/stderr
- Resolve decision with choice 1 → verify pipeline resumes
- Resolve decision with choice 3 → verify pipeline cancelled

## Key Implementation Details

### Decision ID Linking

The `DecisionCreated` event sets `StepStatus::Waiting(Some(decision_id))` in state. This links the pipeline's waiting status to the specific decision. When querying pipeline status, the UI can fetch the decision details.

### Notification Deduplication

The `Effect::Notify` in escalation now describes the decision rather than the raw trigger. The notification message includes:
- Pipeline name
- Trigger type (idle, error, gate)
- Brief summary

Desktop notifications are still sent via `NotifyAdapter`.

### Backward Compatibility

The `StepWaiting` event still exists for non-decision waiting states (e.g., manual pause). Escalations now emit `DecisionCreated` instead, which internally sets `Waiting(Some(id))`. Old WAL entries with bare `StepWaiting` events (no `decision_id`) continue to work — they set `Waiting(None)`.

### Option Numbering

Options are 1-indexed in the CLI and resolution API. This matches user expectations (`oj decision resolve abc123 1` picks the first option). The `DecisionOption` struct doesn't store the number — it's derived from array position during display.

### Source Enum Mapping

| Trigger | DecisionSource |
|---------|----------------|
| on_idle | Idle |
| on_dead | Error |
| on_error | Error |
| gate_failed | Gate |
| on_prompt | Approval |

### Agent Log Tail

For `on_idle`, `on_dead`, and `on_error`, include the last 50 lines of agent output. This provides context for debugging. The log tail is fetched from the step logger, not directly from the tmux session (avoiding race conditions).

## Verification Plan

1. **Unit tests:** `cargo test -p oj-engine` — decision builder tests
2. **Integration tests:** `cargo test -p oj-engine --test runtime_tests` — escalation flow
3. **State tests:** `cargo test -p oj-storage` — DecisionCreated from escalation applies correctly
4. **Full suite:** `cargo test --all`
5. **Clippy + fmt:** `cargo clippy --all -- -D warnings && cargo fmt --all`
6. **Manual smoke test:**
   ```bash
   # Create a pipeline with on_idle = "escalate"
   oj run my-pipeline
   # Wait for idle timeout...
   oj decisions              # See pending decision
   oj decision show <id>     # See context + options
   oj decision resolve <id> 1  # Nudge
   # Verify agent receives nudge message

   # Test gate failure
   # Create pipeline with gate that fails
   oj run gate-test-pipeline
   oj decisions              # See gate failure decision
   oj decision show <id>     # See command + stderr
   oj decision resolve <id> 3  # Cancel
   # Verify pipeline cancelled
   ```
7. **Full CI:** `make check`
