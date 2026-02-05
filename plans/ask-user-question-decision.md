# AskUserQuestion Decision — Show Actual Question & Options

## Overview

When an agent calls Claude Code's `AskUserQuestion` tool, the decision system currently shows generic "Approve / Deny / Cancel" options (the `Approval` source) with no question content. This is because:

1. The PreToolUse hook only captures `tool_name`, discarding `tool_input` (the question data)
2. The prompt type distinction (`Question` vs `Permission`) is lost during escalation — `monitor.rs` maps all prompt triggers to `EscalationTrigger::Prompt { prompt_type: "permission" }`
3. The decision builder has no `Question` variant, so all prompts get `DecisionSource::Approval` with Approve/Deny/Cancel options

This plan threads the actual AskUserQuestion data (question text, options with labels/descriptions, multiSelect flag) through the hook → event → decision job so that `oj decision show` and `oj status` display the real question and choices.

## Project Structure

Files to modify:

```
crates/cli/src/commands/agent.rs           # Capture tool_input from PreToolUse hook JSON
crates/core/src/event.rs                   # Add question_data to AgentPrompt event
crates/engine/src/decision_builder.rs      # Add Question trigger variant with rich data
crates/engine/src/monitor.rs               # Propagate prompt_type → escalation trigger
crates/daemon/src/listener/decisions.rs    # Handle Question decision resolution
crates/daemon/src/protocol_status.rs       # Add question summary to escalated display
```

Test files:

```
crates/engine/src/decision_builder_tests.rs  # Question trigger tests
crates/cli/src/commands/agent_tests.rs       # PreToolUse parsing tests (if exists)
crates/daemon/src/listener/decisions_tests.rs # Question resolution tests
```

## Dependencies

No new external dependencies. Uses existing:
- `serde_json::Value` for tool_input parsing
- `uuid` for decision ID generation

## Implementation Phases

### Phase 1: Capture AskUserQuestion Data in PreToolUse Hook

**Goal:** Parse `tool_input` from the PreToolUse hook JSON and include question data in the `AgentPrompt` event.

**1a. Extend `PreToolUseInput` in `crates/cli/src/commands/agent.rs`:**

The Claude Code PreToolUse hook sends `tool_input` alongside `tool_name` on stdin. Currently only `tool_name` is parsed.

```rust
#[derive(Deserialize)]
struct PreToolUseInput {
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: Option<serde_json::Value>,
}
```

**1b. Define `QuestionData` type in `crates/core/src/event.rs`:**

```rust
/// Structured data from an AskUserQuestion tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionData {
    pub questions: Vec<QuestionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionEntry {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}
```

**1c. Add optional `question_data` to `AgentPrompt` event:**

```rust
#[serde(rename = "agent:prompt")]
AgentPrompt {
    agent_id: AgentId,
    #[serde(default = "default_prompt_type")]
    prompt_type: PromptType,
    /// Populated when prompt_type is Question — contains the actual question and options
    #[serde(default, skip_serializing_if = "Option::is_none")]
    question_data: Option<QuestionData>,
},
```

**1d. Update `handle_pretooluse_hook` to parse and forward question data:**

```rust
async fn handle_pretooluse_hook(agent_id: &str, client: &DaemonClient) -> Result<()> {
    let mut input_json = String::new();
    io::stdin().read_to_string(&mut input_json)?;

    let input: PreToolUseInput =
        serde_json::from_str(&input_json).unwrap_or(PreToolUseInput {
            tool_name: None,
            tool_input: None,
        });

    let Some(prompt_type) = prompt_type_for_tool(input.tool_name.as_deref()) else {
        return Ok(());
    };

    // Extract question data from AskUserQuestion tool_input
    let question_data = if prompt_type == PromptType::Question {
        input.tool_input
            .as_ref()
            .and_then(|v| serde_json::from_value::<QuestionData>(v.clone()).ok())
    } else {
        None
    };

    let event = Event::AgentPrompt {
        agent_id: AgentId::new(agent_id),
        prompt_type,
        question_data,
    };
    client.emit_event(event).await?;

    Ok(())
}
```

**Verification:** Build and confirm the PreToolUse hook parses AskUserQuestion tool_input without breaking existing Permission/PlanApproval flows. The `question_data` field is `None` for non-Question prompts.

### Phase 2: Propagate PromptType Through Escalation

**Goal:** Distinguish Question from Permission prompts all the way from `MonitorState` through `EscalationTrigger` to `DecisionSource`.

**2a. Add `Question` variant to `EscalationTrigger` in `crates/engine/src/decision_builder.rs`:**

```rust
pub enum EscalationTrigger {
    Idle,
    Dead { exit_code: Option<i32> },
    Error { error_type: String, message: String },
    GateFailed { command: String, exit_code: i32, stderr: String },
    /// Agent showed a permission prompt (on_prompt with permission type)
    Prompt { prompt_type: String },
    /// Agent called AskUserQuestion — carries the parsed question data
    Question { question_data: Option<QuestionData> },
}
```

Update `to_source()`:

```rust
impl EscalationTrigger {
    pub fn to_source(&self) -> DecisionSource {
        match self {
            EscalationTrigger::Idle => DecisionSource::Idle,
            EscalationTrigger::Dead { .. } => DecisionSource::Error,
            EscalationTrigger::Error { .. } => DecisionSource::Error,
            EscalationTrigger::GateFailed { .. } => DecisionSource::Gate,
            EscalationTrigger::Prompt { .. } => DecisionSource::Approval,
            EscalationTrigger::Question { .. } => DecisionSource::Question,
        }
    }
}
```

**2b. Update `build_action_effects` in `crates/engine/src/monitor.rs` to use prompt type:**

Currently `"prompt" | "on_prompt"` always maps to `EscalationTrigger::Prompt`. Change to inspect the `PromptType` from `MonitorState`:

The `build_action_effects` function receives `trigger: &str` which loses the `PromptType`. Add the prompt type as context. The simplest approach: encode prompt type in the trigger string.

In `handle_monitor_state` (runtime/monitor.rs):

```rust
MonitorState::Prompting { ref prompt_type } => {
    // Use distinct trigger strings so escalation can differentiate
    let trigger_str = match prompt_type {
        PromptType::Question => "prompt:question",
        PromptType::Permission => "prompt:permission",
        PromptType::PlanApproval => "prompt:plan_approval",
        _ => "prompt",
    };
    (&agent_def.on_prompt, trigger_str)
}
```

Then in `build_action_effects` (monitor.rs, `AgentAction::Escalate` arm):

```rust
"prompt:question" => EscalationTrigger::Question {
    question_data: None, // filled below from event context
},
"prompt:permission" | "prompt:plan_approval" | "prompt" | "on_prompt" => {
    EscalationTrigger::Prompt {
        prompt_type: "permission".to_string(),
    }
}
```

**2c. Thread `QuestionData` from the event to the escalation trigger:**

The `QuestionData` lives on the `AgentPrompt` event. It needs to reach `build_action_effects`. Options:

- **Option A (recommended):** Add an optional `question_data: Option<QuestionData>` parameter to `build_action_effects` and plumb it through `handle_monitor_state` → `execute_action_with_attempts`. The `handle_agent_prompt_hook` method in `runtime/handlers/agent.rs` already receives the full event with `question_data`.

- **Option B:** Store `question_data` in `MonitorState::Prompting` and extract it in the monitor.

Choose Option A. Add a `question_data` parameter (defaulting to `None`) to `build_action_effects`:

```rust
pub fn build_action_effects(
    job: &Job,
    agent_def: &AgentDef,
    action_config: &ActionConfig,
    trigger: &str,
    input: &HashMap<String, String>,
    question_data: Option<&QuestionData>,  // NEW
) -> Result<ActionEffects, RuntimeError>
```

Existing call sites pass `None`. The prompt handler in `runtime/handlers/agent.rs` passes through the question data from the event.

**Verification:** Run `cargo test -p oj-engine`. Confirm Permission prompts still create `DecisionSource::Approval` decisions while Question prompts create `DecisionSource::Question` decisions.

### Phase 3: Build Question-Specific Decision Options

**Goal:** When the escalation trigger is `Question`, build decision options from the AskUserQuestion data instead of using generic Approve/Deny/Cancel.

**3a. Add `build_options` arm for `Question` in `decision_builder.rs`:**

```rust
EscalationTrigger::Question { ref question_data } => {
    let mut options = Vec::new();

    if let Some(qd) = question_data {
        // Use the first question's options (most common case: single question)
        if let Some(entry) = qd.questions.first() {
            for opt in &entry.options {
                options.push(DecisionOption {
                    label: opt.label.clone(),
                    description: opt.description.clone(),
                    recommended: false,
                });
            }
        }
    }

    // Always add a "Type something" option for freeform answers
    // (mirrors the implicit "Other" in AskUserQuestion)
    // Users can also provide freeform via -m flag without a choice number

    // Always add Cancel as the last option
    options.push(DecisionOption {
        label: "Cancel".to_string(),
        description: Some("Cancel the job".to_string()),
        recommended: false,
    });

    options
}
```

**3b. Add `build_context` for `Question` trigger:**

```rust
EscalationTrigger::Question { ref question_data } => {
    if let Some(qd) = question_data {
        if let Some(entry) = qd.questions.first() {
            let header = entry.header.as_deref().unwrap_or("Question");
            parts.push(format!(
                "Agent in job \"{}\" is asking a question.",
                self.job_name
            ));
            parts.push(String::new()); // blank line
            parts.push(format!("[{}] {}", header, entry.question));

            // If there are multiple questions, list them all
            for (i, q) in qd.questions.iter().enumerate().skip(1) {
                let h = q.header.as_deref().unwrap_or("Question");
                parts.push(format!("[{}] {}", h, q.question));
            }
        } else {
            parts.push(format!(
                "Agent in job \"{}\" is asking a question.",
                self.job_name
            ));
        }
    } else {
        parts.push(format!(
            "Agent in job \"{}\" is asking a question (no details available).",
            self.job_name
        ));
    }
}
```

**Verification:** Create a decision via the builder with `EscalationTrigger::Question` and verify the context includes the question text and options match the AskUserQuestion data.

### Phase 4: Update Decision Resolution for Questions

**Goal:** When a Question decision is resolved, format the answer so the agent receives the selected option.

**4a. Update `map_decision_to_action` in `crates/daemon/src/listener/decisions.rs`:**

The existing `DecisionSource::Question` handler already does `JobResume`. Enhance it to include the selected option label in the message, making it actionable for the agent:

```rust
DecisionSource::Question => {
    // Check if last option was chosen (Cancel — always appended last)
    let options = &state_guard
        .get_decision(id)
        .map(|d| d.options.clone())
        .unwrap_or_default();

    if let Some(c) = chosen {
        // Last option is always Cancel
        if c == options.len() {
            return Some(Event::JobCancel { id: pid });
        }
    }

    // For non-Cancel choices: resume with the selected option info
    Some(Event::JobResume {
        id: pid,
        message: Some(build_question_resume_message(chosen, message, decision_id, options)),
        vars: HashMap::new(),
    })
}
```

**4b. Add `build_question_resume_message` helper:**

The message sent to the tmux session should select the option in Claude Code's interactive AskUserQuestion picker. The picker UI accepts arrow key navigation and typed text to filter.

```rust
fn build_question_resume_message(
    chosen: Option<usize>,
    message: Option<&str>,
    decision_id: &str,
    options: &[DecisionOption],
) -> String {
    let mut parts = Vec::new();

    if let Some(c) = chosen {
        // Get the label of the chosen option
        let label = options
            .get(c - 1) // 1-indexed to 0-indexed
            .map(|o| o.label.as_str())
            .unwrap_or("unknown");
        parts.push(format!("Selected: {} (option {})", label, c));
    }
    if let Some(m) = message {
        parts.push(m.to_string());
    }
    if parts.is_empty() {
        parts.push(format!("decision {} resolved", decision_id));
    }

    parts.join("; ")
}
```

**4c. Keep the options list accessible during resolution:**

The current `handle_decision_resolve` reads the decision from state, extracts `source` and `job_id`, then drops the state lock. For Question decisions, we also need the `options` list to determine if the chosen option is Cancel (last option). Read `options` before dropping the lock:

```rust
let decision_options = decision.options.clone();
// ... later in map_decision_to_action, use decision_options
```

**Verification:** Resolve a Question decision with choice 1, verify `JobResume` emits with the option label. Resolve with the last choice number, verify `JobCancel` emits.

### Phase 5: Update Status Display

**Goal:** Show "question" as the escalation source in `oj status` and enrich `oj decision show` for Question-type decisions.

**5a. The `escalate_source` field in `JobStatusEntry` already exists and is populated from `DecisionSource`.**

With Phase 2 routing AskUserQuestion to `DecisionSource::Question`, the status display will automatically show `[question]` instead of `[approval]`. No changes needed in `protocol_status.rs` or `status.rs` — the source label derives from the `DecisionSource` enum's serialization (`"question"`).

**5b. Update `oj decision show` display to show question text prominently:**

In `crates/cli/src/commands/decision.rs`, the `Context:` section already prints the full decision context. With the rich context from Phase 3, the question text and options will appear naturally. No additional changes needed — the context string already contains the formatted question.

**5c. (Optional) Add question-specific hints in the resolve prompt:**

When showing an unresolved Question decision, hint that the user can type a freeform answer:

```
Use: oj decision resolve abc12345 <number> [-m message]
Tip: Use -m to provide a custom answer (option "Other")
```

**Verification:** Run `oj status` with a Question decision pending, confirm it shows `[question]` source. Run `oj decision show`, confirm the question text and options appear.

### Phase 6: Clean Up Prompt vs Question Distinction

**Goal:** Remove the ambiguity between Permission and Question prompts.

**6a. Audit PromptType usage:**

The `PromptType` enum has: `Permission`, `Idle`, `PlanApproval`, `Question`, `Other`. After this change:
- `Permission` → `EscalationTrigger::Prompt` → `DecisionSource::Approval` (Approve/Deny/Cancel)
- `Question` → `EscalationTrigger::Question` → `DecisionSource::Question` (actual options + Cancel)
- `PlanApproval` → `EscalationTrigger::Prompt` → `DecisionSource::Approval` (Approve/Deny/Cancel) — plan mode approval is semantically similar to permission approval

This is clean. No changes needed to `PromptType` or `DecisionSource` enums.

**6b. Update existing tests:**

- `decision_builder_tests.rs`: Add tests for `Question` trigger
- `decisions_tests.rs`: Add tests for Question resolution → JobResume with label
- Any tests that assert on the exact trigger mapping for "prompt" trigger strings

**Verification:** `make check` passes. All existing tests pass. New tests cover Question decision creation and resolution.

## Key Implementation Details

### Data Flow

```
Claude Code agent calls AskUserQuestion
  → PreToolUse hook fires on stdin with tool_name + tool_input
  → oj agent hook pretooluse <id> reads stdin
  → Parses QuestionData from tool_input.questions
  → Emits Event::AgentPrompt { prompt_type: Question, question_data: Some(...) }
  → Daemon processes event
  → Runtime: handle_agent_prompt_hook receives prompt_type + question_data
  → MonitorState::Prompting { prompt_type: Question }
  → on_prompt action (default: escalate)
  → build_action_effects with trigger "prompt:question"
  → EscalationTrigger::Question { question_data }
  → EscalationDecisionBuilder builds DecisionCreated event
    → source: DecisionSource::Question
    → context: "Agent is asking: [Header] Question text?"
    → options: [Option1, Option2, ..., Cancel]
  → Decision stored in MaterializedState
  → Desktop notification: "Decision needed: job-name"
  → oj status shows [question] source
  → oj decision show shows question + options

User resolves:
  → oj decision resolve <id> 2 -m "custom note"
  → handle_decision_resolve validates choice
  → Emits DecisionResolved
  → map_decision_to_action:
    → If last option (Cancel) → JobCancel
    → Otherwise → JobResume with "Selected: <label> (option N); custom note"
  → Agent receives message via tmux session
```

### WAL Backward Compatibility

The `question_data` field on `Event::AgentPrompt` uses `#[serde(default, skip_serializing_if = "Option::is_none")]`, so old WAL entries without this field deserialize correctly as `None`. Older events with `AgentPrompt` that lack `question_data` continue to work — they follow the existing Permission/Approval path since they'll have `prompt_type: Permission`.

### Edge Case: Agent Answers in tmux Directly

If the user attaches to the tmux session and answers the AskUserQuestion interactively (bypassing `oj decision resolve`), the decision remains pending. The agent will transition from `Prompting` to `Working`, which triggers the liveness timer reset. The stale decision should be auto-dismissed.

This is not addressed in this plan — it's an existing limitation of the decision system (decisions don't auto-resolve when the agent moves on). A follow-up could add auto-resolution when the agent state changes from `Prompting` to `Working`.

### Multi-Question AskUserQuestion

AskUserQuestion supports 1-4 questions per call. This plan handles the first question's options as the decision options. For multi-question calls, the context will list all questions but only the first question's options become the decision options. This covers the common case (single question). Multi-question support can be added later if needed.

### Cancel Option Position

Cancel is always appended as the last option. The `map_decision_to_action` function checks if the chosen option index equals the options length (i.e., the Cancel option). This differs from the hardcoded position 3 used by Approval/Error/Gate decisions. The resolution logic needs to handle this dynamic positioning.

### Option Numbering

Options are 1-indexed in the CLI. The AskUserQuestion options map to positions 1..N, and Cancel is at position N+1. Example for a 3-option question:

```
Options:
  1. Unreachable step - "step 'X' is unreachable..."
  2. Dead-end step - "step 'X' has no on_done..."
  3. Both - Remove both parser warnings
  4. Cancel - Cancel the job

Use: oj decision resolve abc12345 <number> [-m message]
```

## Verification Plan

1. **Unit tests:** `cargo test -p oj-engine` — decision builder Question trigger
2. **Unit tests:** `cargo test -p oj-daemon` — Question decision resolution
3. **Build check:** `cargo build --all`
4. **Lint check:** `cargo clippy --all -- -D warnings && cargo fmt --all`
5. **Full suite:** `cargo test --all`
6. **Manual smoke test:**
   ```bash
   # Create a job with an agent that calls AskUserQuestion
   oj run test-job
   # When agent asks a question...
   oj status                     # See [question] source on escalated job
   oj decision list              # See pending question decision
   oj decision show <id>         # See actual question text + options
   oj decision resolve <id> 1    # Select first option
   # Verify agent receives the selection
   ```
7. **Full CI:** `make check`
