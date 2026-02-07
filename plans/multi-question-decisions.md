# Multi-Question Decision Support

## Context

Claude Code's `AskUserQuestion` tool sends 1-4 questions per invocation, each with their own options. Currently, oddjobs only extracts the **first question's options** for the decision — options from questions 2+ are silently dropped. All question text appears in the decision context, but the user can only pick from Q1's options and only one answer gets sent to the terminal.

The fix: store full question data on the decision, accept per-question answers, and route them all to the terminal.

## TUI Behavior (verified via claudeless source)

The AskUserQuestion dialog is a **tabbed TUI widget**, not line-based input:

- Questions are displayed one at a time with a tab bar: `← ☐ Q1  ☒ Q2  [✔ Submit] →`
- Pressing a digit (`1`-`9`) selects that option for the current question
- For single-select, selecting auto-advances to the next question (or Submit tab)
- On the Submit tab, `1` confirms, `2` cancels, or Enter confirms at cursor position
- Answers are collected as `HashMap<question_text, option_label>`

**Terminal input for answering:**
- Single question (2 options), pick option 2: `"2\n"` — digit selects + advances to Submit, Enter confirms
- Two questions, pick option 1 then option 2: `"12\n"` — digits select + advance, Enter confirms
- Three questions, pick 1, 3, 2: `"132\n"` — same pattern

The existing single-question code (`format!("{}\n", chosen)`) already works correctly. For multi-question, we concatenate per-question digits and end with `\n`.

## Approach

- Add `question_data: Option<QuestionData>` to `Decision` and `DecisionCreated` event (for grouping/display)
- Add `choices: Vec<usize>` to `Decision`, `DecisionResolved` event, and `DecisionResolve` IPC request (per-question 1-indexed answers)
- Build options from ALL questions (not just first) in the decision builder
- Display options grouped by question in CLI
- Accept multiple positional args in `oj decision resolve`: `oj decision resolve <id> 1 2`
- Route multi-answer to terminal as concatenated digits: `"12\n"`

All new serde fields use `#[serde(default)]` — fully backward compatible with existing WAL entries.

## Changes

### 1. Core data model (DONE)

**`crates/core/src/decision.rs`** — Added to `Decision`:
```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub question_data: Option<QuestionData>,
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub choices: Vec<usize>,
```

**`crates/core/src/event/mod.rs`** — Added to `DecisionCreated`:
```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
question_data: Option<QuestionData>,
```

Added to `DecisionResolved`:
```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
choices: Vec<usize>,
```

### 2. Storage state

**`crates/storage/src/state/decisions.rs`**

`DecisionCreated` handler: store `question_data` on the `Decision`.
`DecisionResolved` handler: store `choices` on the `Decision`.

### 3. Decision builder

**`crates/engine/src/decision_builder.rs`**

`build_options()` for `Question` trigger: iterate ALL questions' options, not just first:
```rust
for entry in &qd.questions {
    for opt in &entry.options { ... }
}
options.push(DecisionOption::new("Cancel")...);
```

`build()` method: pass `question_data` from the trigger into the `DecisionCreated` event. Extract it from `EscalationTrigger::Question { question_data, .. }`.

`build_context()`: already handles multiple questions — no change needed.

### 4. Protocol types

**`crates/daemon/src/protocol.rs`** — Add to `DecisionResolve`:
```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
choices: Vec<usize>,
```

**`crates/daemon/src/protocol_types.rs`** — Add to `DecisionDetail`:
```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub question_groups: Vec<QuestionGroupDetail>,
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub choices: Vec<usize>,
```

New struct:
```rust
pub struct QuestionGroupDetail {
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<DecisionOptionDetail>,
}
```

### 5. Query handler

**`crates/daemon/src/listener/query.rs`** — `Query::GetDecision`:
Build `question_groups` from `decision.question_data` when present. Include `choices` from `decision.choices`.

### 6. Decision resolution handler

**`crates/daemon/src/listener/decisions.rs`**

`handle_decision_resolve()`:
- Accept `choices` from request
- Validate: if `choices` non-empty, count must match question count; each choice in range for its question's options
- Pass `choices` into `DecisionResolved` event

`resolve_decision_action()`:
- Unchanged — still uses `chosen` for single-question legacy path
- When `choices` is non-empty, the caller maps to `ResolvedAction::Answer`

Answer routing for agent runs (tmux):
```rust
// Multi-question: per-question digits concatenated + Enter
// e.g., choices [1, 2] → "12\n"
let input = choices.iter().map(|c| c.to_string()).collect::<String>() + "\n";
send_to_session(input)
```

Answer routing for jobs (`JobResume`):
```rust
// Human-readable: "Q1: React (1); Q2: PostgreSQL (2)"
build_multi_question_resume_message(choices, question_data, decision_id)
```

**`crates/daemon/src/listener/mod.rs`** — Update `DecisionResolve` destructure to pass `choices`.

### 7. CLI commands

**`crates/cli/src/commands/decision.rs`**

`Resolve` command: change `choice: Option<usize>` to `choice: Vec<usize>`:
```rust
Resolve {
    id: String,
    /// Pick option(s) — one per question for multi-question decisions
    choice: Vec<usize>,
    #[arg(short = 'm', long)]
    message: Option<String>,
}
```

Usage: `oj decision resolve <id> 1` (single-question), `oj decision resolve <id> 1 2` (multi-question).

`format_decision_detail()`: when `question_groups` is non-empty, display grouped:
```
Question 1 [Framework]: Which framework?
  1. React - Component-based UI
  2. Vue - Progressive framework

Question 2 [Database]: Which database?
  1. PostgreSQL
  2. MySQL

  C. Cancel - Cancel and fail

Use: oj decision resolve abc12345 <q1> <q2>
```

When `question_groups` is empty, display flat options as today.

### 8. CLI client

**`crates/cli/src/client_queries_queue.rs`** — `decision_resolve()`: accept `choices` vec, pass through to `DecisionResolve` request. Map CLI args: if single element, use `chosen` for backward compat; if multiple elements, use `choices`.

### 9. Tests

- **`crates/engine/src/decision_builder_tests.rs`**: Update `test_question_trigger_multi_question_context` to verify options from all questions appear. Add test for multi-question option building.
- **`crates/daemon/src/listener/decisions_tests.rs`**: Add tests for multi-question resolution, validation, and answer routing.
- **`crates/cli/src/commands/decision_tests.rs`**: Add display test for grouped question output.
- **`crates/core/src/decision_tests.rs`**: Add serde roundtrip test for new fields.

### 10. `make check`

## Verification

1. `cargo build --all` — compiles clean
2. `cargo test --all` — all existing tests pass
3. `oj decision show` displays grouped questions
4. `oj decision resolve <id> 1 2` sends both answers to terminal
5. Single-question decisions still work identically (`oj decision resolve <id> 1`)
6. `make check` passes (fmt, clippy, quench, build, test, deny)
