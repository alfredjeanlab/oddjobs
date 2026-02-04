# Plan: Show Escalation Reason in `oj status`

## Overview

Add the escalation source label (e.g., `idle`, `error`, `gate`, `approval`) to the escalated pipeline entries in `oj status`, matching the pattern used in `oj decision list` which already shows a `SOURCE` column. Currently, escalated pipelines only show the verbose `waiting_reason` text; the concise source category is missing.

## Project Structure

Files to modify:

```
crates/daemon/src/protocol_status.rs   # Add `escalate_source` field to PipelineStatusEntry
crates/daemon/src/listener/query.rs    # Look up decision source when building escalated entries
crates/cli/src/commands/status.rs      # Display the escalate source in output
crates/cli/src/commands/status_tests.rs # Update/add tests for new field
```

## Dependencies

No new external dependencies. Uses existing `DecisionSource` from `oj_core` and `Decision` from `MaterializedState`.

## Implementation Phases

### Phase 1: Add `escalate_source` field to `PipelineStatusEntry`

**File:** `crates/daemon/src/protocol_status.rs`

Add an optional `escalate_source` field to `PipelineStatusEntry`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineStatusEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub step: String,
    pub step_status: String,
    pub elapsed_ms: u64,
    pub waiting_reason: Option<String>,
    /// Escalation source category (e.g., "idle", "error", "gate", "approval")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalate_source: Option<String>,
}
```

**Verification:** `cargo build --all` passes.

### Phase 2: Populate `escalate_source` from decision state

**File:** `crates/daemon/src/listener/query.rs`

In the `Query::StatusOverview` handler (around line 919-948), when building `PipelineStatusEntry` for pipelines with `step_status.is_waiting()`, look up the decision source:

1. Extract the `decision_id` from `StepStatus::Waiting(Some(decision_id))`.
2. Look up the decision in `state.decisions` by that ID.
3. Format the `DecisionSource` as a lowercase string (using the same `format!("{:?}", d.source).to_lowercase()` pattern used in `Query::ListDecisions` at line 1105).

```rust
let escalate_source = match &p.step_status {
    StepStatus::Waiting(Some(decision_id)) => {
        state.decisions.get(decision_id.as_str()).map(|d| {
            format!("{:?}", d.source).to_lowercase()
        })
    }
    _ => None,
};

let entry = PipelineStatusEntry {
    id: p.id.clone(),
    name: p.name.clone(),
    kind: p.kind.clone(),
    step: p.step.clone(),
    step_status: p.step_status.to_string(),
    elapsed_ms,
    waiting_reason,
    escalate_source,
};
```

Note: The `state` variable is the locked `MaterializedState` which has both `pipelines` and `decisions` accessible within the same lock scope. `StepStatus` is already imported in this module.

**Verification:** `cargo build --all` passes, `cargo test --all` passes (existing tests should still work since `escalate_source` defaults to `None` via serde).

### Phase 3: Display `escalate_source` in CLI status output

**File:** `crates/cli/src/commands/status.rs`

In the escalated pipelines rendering block (lines 288-305), show the source label after the warning icon. The display should look like:

```
  Escalated (1):
    ⚠ abcd1234  deploy deploy-staging  test  waiting  [idle]  1m
      → Agent in pipeline "deploy-staging" is idle and waiting for input.
```

Updated rendering code:

```rust
for p in &ns.escalated_pipelines {
    let short_id = truncate_id(&p.id, 8);
    let elapsed = format_duration_ms(p.elapsed_ms);
    let friendly = friendly_name_label(&p.name, &p.kind, &p.id);
    let source_label = p.escalate_source
        .as_deref()
        .map(|s| format!("[{}]  ", s))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "    {} {}  {}{}  {}  {}  {}{}",
        color::yellow("⚠"),
        color::muted(short_id),
        p.kind,
        friendly,
        p.step,
        color::status(&p.step_status),
        source_label,
        elapsed,
    );
    if let Some(ref reason) = p.waiting_reason {
        let _ = writeln!(out, "      → {}", truncate_reason(reason, 72));
    }
}
```

The source label is displayed in brackets `[idle]` before the elapsed time, keeping it visually distinct from the other fields. When no source is available (legacy pipelines without a decision), it's omitted gracefully.

**Verification:** `cargo build --all` passes, visual inspection of `oj status` output with escalated pipelines.

### Phase 4: Update tests

**File:** `crates/cli/src/commands/status_tests.rs`

1. **Update existing test constructors**: Add `escalate_source: None` (or `Some("idle".into())` for escalated entries) to all `PipelineStatusEntry` literals in tests. Due to the `#[serde(default)]` attribute, this is only needed for tests that construct `PipelineStatusEntry` directly.

2. **Add a new test** for the escalate source display:

```rust
#[test]
#[serial]
fn escalated_pipeline_shows_source_label() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "efgh5678-0000-0000-0000".to_string(),
            name: "deploy-staging-efgh5678".to_string(),
            kind: "deploy".to_string(),
            step: "test".to_string(),
            step_status: "waiting".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: Some("Agent is idle".to_string()),
            escalate_source: Some("idle".to_string()),
        }],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("[idle]"),
        "output should contain source label '[idle]':\n{output}"
    );
}
```

3. **Add a test** confirming no source label when `escalate_source` is `None`:

```rust
#[test]
#[serial]
fn escalated_pipeline_no_source_label_when_none() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "efgh5678-0000-0000-0000".to_string(),
            name: "deploy-staging-efgh5678".to_string(),
            kind: "deploy".to_string(),
            step: "test".to_string(),
            step_status: "waiting".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: Some("gate check failed".to_string()),
            escalate_source: None,
        }],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    assert!(
        !output.contains("["),
        "output should not contain bracket source label when source is None:\n{output}"
    );
}
```

**Verification:** `cargo test --all` passes, all new and existing tests green.

## Key Implementation Details

### Data Flow

```
Pipeline.step_status: StepStatus::Waiting(Some(decision_id))
                ↓
state.decisions.get(decision_id) → Decision { source: DecisionSource::Idle, ... }
                ↓
format!("{:?}", source).to_lowercase() → "idle"
                ↓
PipelineStatusEntry { escalate_source: Some("idle"), ... }
                ↓
CLI: "⚠ abcd1234  deploy  test  waiting  [idle]  1m"
```

### Why `Option<String>` instead of reusing `DecisionSource`

The protocol types in `crates/daemon/src/protocol_status.rs` use simple serializable types (`String`, `u64`, etc.) rather than core domain types. This keeps the IPC layer decoupled from core internals and maintains backward compatibility (old daemons can communicate with new CLIs and vice versa via `#[serde(default)]`).

### Backward Compatibility

- `#[serde(default, skip_serializing_if = "Option::is_none")]` on the new field ensures old daemons (that don't send `escalate_source`) still work with new CLIs (field defaults to `None`).
- Old CLIs ignore unknown fields in JSON deserialization.

### Source Label Values

Matches the existing `DecisionSource` enum values (via `Debug` + `to_lowercase()`):
- `idle` — agent was idle (on_idle escalation)
- `error` — agent died or hit an error (on_dead/on_error escalation)
- `gate` — gate command failed
- `approval` — agent showed a permission prompt
- `question` — agent asked a question (not from escalation, but possible)

## Verification Plan

1. **Unit tests**: Phase 4 tests verify the CLI rendering logic.
2. **Build check**: `cargo build --all` after each phase.
3. **Full CI**: `make check` after Phase 4 (fmt, clippy, build, test, deny).
4. **Manual verification**: Start a pipeline, trigger an escalation (e.g., via `on_idle = "escalate"`), and confirm `oj status` output shows `[idle]` next to the escalated entry.
