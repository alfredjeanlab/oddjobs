# Decision System — Phase 1: Data Model, Storage & CLI

## Overview

Add a first-class decision entity to the system. Decisions represent points where a job is blocked and needs human input — gate failures, agent questions, idle escalations, errors, or approval requests. Phase 1 establishes the data model, event sourcing, materialized state, daemon protocol, and CLI commands. Later phases will wire escalation paths and agent hooks to emit decisions automatically.

## Project Structure

New files:
```
crates/core/src/decision.rs          # DecisionId, DecisionSource, DecisionOption, Decision
crates/core/src/decision_tests.rs    # Unit tests for decision types
crates/cli/src/commands/decision.rs  # CLI: oj decision {list,show,resolve}
crates/cli/src/commands/decision_tests.rs
crates/daemon/src/listener/decisions.rs  # Daemon handlers for decision resolve
crates/daemon/src/listener/decisions_tests.rs
```

Modified files:
```
crates/core/src/lib.rs               # pub mod decision; re-exports
crates/core/src/event.rs             # DecisionCreated, DecisionResolved events
crates/core/src/job.rs          # StepStatus::Waiting(String) with decision_id
crates/storage/src/state.rs          # decisions HashMap + apply_event arms
crates/storage/src/state_tests/mod.rs # Tests for decision event application
crates/daemon/src/protocol.rs        # Request/Response/Query variants for decisions
crates/daemon/src/listener/mod.rs    # Wire up decision handlers
crates/daemon/src/listener/query.rs  # Query::ListDecisions, Query::GetDecision handlers
crates/cli/src/commands/mod.rs       # pub mod decision;
crates/cli/src/main.rs               # Decision + Decisions command variants
```

## Dependencies

No new external crate dependencies. Uses existing `uuid`, `serde`, `clap`, `chrono` (or raw epoch_ms).

## Implementation Phases

### Phase 1: Core Data Model (`crates/core`)

**Goal:** Define the decision types and update StepStatus.

**1a. New module `crates/core/src/decision.rs`:**

```rust
use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for a decision.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DecisionId(pub String);

impl DecisionId {
    pub fn new(id: impl Into<String>) -> Self { Self(id.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl fmt::Display for DecisionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// Add From<String>, From<&str>, PartialEq<str>, Borrow<str> impls
// following JobId pattern in crates/core/src/job.rs

/// Where the decision originated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    Question,
    Approval,
    Gate,
    Error,
    Idle,
}

/// A single option the user can choose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub recommended: bool,
}

/// A decision awaiting (or resolved by) human input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: DecisionId,
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub source: DecisionSource,
    pub context: String,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    /// 1-indexed choice (None = unresolved or freeform-only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen: Option<usize>,
    /// Freeform message from the resolver
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_ms: Option<u64>,
    #[serde(default)]
    pub namespace: String,
}

impl Decision {
    pub fn is_resolved(&self) -> bool {
        self.resolved_at_ms.is_some()
    }
}
```

**1b. Update `StepStatus` in `crates/core/src/job.rs`:**

Change `Waiting` from a unit variant to carry an optional decision ID:

```rust
pub enum StepStatus {
    Pending,
    Running,
    Waiting(Option<String>),  // Optional decision_id
    Completed,
    Failed,
}
```

Using `Option<String>` rather than `Option<DecisionId>` keeps `StepStatus` simple and avoids a circular dependency issue (since `Copy` would be lost anyway). The `Option` provides backward compat: existing `StepWaiting` events without a decision produce `Waiting(None)`.

**Important:** `StepStatus` currently derives `Copy`. Changing `Waiting` to carry data removes `Copy`. All sites that copy `StepStatus` need updating to use `.clone()`. Audit all uses:
- `crates/storage/src/state.rs` — assignments like `job.step_status = StepStatus::Waiting` → `StepStatus::Waiting(None)` or `StepStatus::Waiting(Some(decision_id))`
- `crates/engine/src/runtime/monitor.rs:652` — comparison
- `crates/daemon/src/lifecycle.rs:626` — comparison
- `crates/daemon/src/listener/query.rs:606` — match arm
- Various test files — assertions

For comparisons, add a helper:

```rust
impl StepStatus {
    pub fn is_waiting(&self) -> bool {
        matches!(self, StepStatus::Waiting(_))
    }
}
```

**1c. Update `crates/core/src/lib.rs`:**

```rust
pub mod decision;
pub use decision::{Decision, DecisionId, DecisionOption, DecisionSource};
```

**1d. Update `StepOutcome::Waiting`:**

`StepOutcome::Waiting(String)` currently carries a reason string. Keep this as-is — it's used for step history display and the reason string is separate from the decision ID.

### Phase 2: Events (`crates/core/src/event.rs`)

**Goal:** Add DecisionCreated and DecisionResolved event variants.

```rust
// In the Event enum:

#[serde(rename = "decision:created")]
DecisionCreated {
    id: String,
    job_id: JobId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    source: DecisionSource,
    context: String,
    #[serde(default)]
    options: Vec<DecisionOption>,
    created_at_ms: u64,
    #[serde(default)]
    namespace: String,
},

#[serde(rename = "decision:resolved")]
DecisionResolved {
    id: String,
    /// 1-indexed choice picking a numbered option
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chosen: Option<usize>,
    /// Freeform text (nudge message, custom answer)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    resolved_at_ms: u64,
    #[serde(default)]
    namespace: String,
},
```

Also update:
- `Event::name()` — add `"decision:created"` and `"decision:resolved"` arms
- `Event::log_summary()` — add summary lines, e.g. `"decision:created id={id} job={job_id} source={source:?}"`
- `Event::job_id()` — return `Some(job_id)` for `DecisionCreated`

Import `DecisionSource` and `DecisionOption` from `crate::decision`.

### Phase 3: Materialized State (`crates/storage/src/state.rs`)

**Goal:** Store decisions in state, apply events.

**3a. Add decisions field to `MaterializedState`:**

```rust
pub struct MaterializedState {
    // ... existing fields ...
    #[serde(default)]
    pub decisions: HashMap<String, Decision>,
}
```

Import `Decision` from `oj_core`.

**3b. Add `apply_event` arms:**

```rust
Event::DecisionCreated {
    id,
    job_id,
    agent_id,
    source,
    context,
    options,
    created_at_ms,
    namespace,
} => {
    // Idempotency: skip if already exists
    if !self.decisions.contains_key(id) {
        self.decisions.insert(id.clone(), Decision {
            id: DecisionId::new(id.clone()),
            job_id: job_id.to_string(),
            agent_id: agent_id.clone(),
            source: source.clone(),
            context: context.clone(),
            options: options.clone(),
            chosen: None,
            message: None,
            created_at_ms: *created_at_ms,
            resolved_at_ms: None,
            namespace: namespace.clone(),
        });
    }

    // Update job step status to Waiting with decision_id
    if let Some(job) = self.jobs.get_mut(job_id.as_str()) {
        job.step_status = StepStatus::Waiting(Some(id.clone()));
    }
},

Event::DecisionResolved {
    id,
    chosen,
    message,
    resolved_at_ms,
    ..
} => {
    if let Some(decision) = self.decisions.get_mut(id) {
        decision.chosen = *chosen;
        decision.message.clone_from(message);
        decision.resolved_at_ms = Some(*resolved_at_ms);
    }
},
```

Note: `DecisionResolved` only updates the decision record. The job resume (transitioning out of `Waiting`) is handled by a separate `JobResume` event emitted by the daemon handler after resolving — this keeps the events composable and avoids coupling decision resolution to job advancement.

**3c. Add `get_decision` helper:**

```rust
impl MaterializedState {
    /// Get a decision by ID or unique prefix
    pub fn get_decision(&self, id: &str) -> Option<&Decision> {
        if let Some(decision) = self.decisions.get(id) {
            return Some(decision);
        }
        let matches: Vec<_> = self.decisions
            .iter()
            .filter(|(k, _)| k.starts_with(id))
            .collect();
        if matches.len() == 1 {
            Some(matches[0].1)
        } else {
            None
        }
    }
}
```

### Phase 4: Daemon Protocol & Handlers

**Goal:** Add IPC protocol types and daemon-side handlers.

**4a. Protocol types (`crates/daemon/src/protocol.rs`):**

Add to `Query`:
```rust
/// List pending decisions (optionally filtered by namespace)
ListDecisions {
    #[serde(default)]
    namespace: String,
},
/// Get a single decision by ID (prefix match supported)
GetDecision {
    id: String,
},
```

Add to `Response`:
```rust
/// List of decisions
Decisions { decisions: Vec<DecisionSummary> },

/// Single decision detail
Decision { decision: Option<Box<DecisionDetail>> },

/// Decision resolved successfully
DecisionResolved { id: String },
```

Add summary/detail structs:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionSummary {
    pub id: String,
    pub job_id: String,
    pub job_name: String,
    pub source: String,
    pub summary: String,       // truncated context
    pub created_at_ms: u64,
    #[serde(default)]
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionDetail {
    pub id: String,
    pub job_id: String,
    pub job_name: String,
    pub agent_id: Option<String>,
    pub source: String,
    pub context: String,
    pub options: Vec<DecisionOptionDetail>,
    pub chosen: Option<usize>,
    pub message: Option<String>,
    pub created_at_ms: u64,
    pub resolved_at_ms: Option<u64>,
    #[serde(default)]
    pub namespace: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionOptionDetail {
    pub number: usize,          // 1-indexed for display
    pub label: String,
    pub description: Option<String>,
    pub recommended: bool,
}
```

Add to `Request`:
```rust
/// Resolve a pending decision
DecisionResolve {
    id: String,
    /// 1-indexed option choice
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chosen: Option<usize>,
    /// Freeform message
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<String>,
},
```

**4b. Query handlers (`crates/daemon/src/listener/query.rs`):**

Add `Query::ListDecisions` and `Query::GetDecision` arms in `handle_query()`. For `ListDecisions`, filter by namespace and return only unresolved decisions (resolved_at_ms.is_none()). Map to `DecisionSummary` with job name looked up from state.

**4c. Mutation handler (`crates/daemon/src/listener/decisions.rs`):**

```rust
pub(super) fn handle_decision_resolve(
    id: &str,
    chosen: Option<usize>,
    message: Option<String>,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    let state_guard = state.lock();

    // Find decision by ID or prefix
    let decision = state_guard.get_decision(id)
        .ok_or_else(|| ConnectionError::Internal(format!("decision not found: {}", id)))?;

    // Validate: must be unresolved
    if decision.is_resolved() {
        return Ok(Response::Error {
            message: format!("decision {} is already resolved", id),
        });
    }

    // Validate: choice must be in range if provided
    if let Some(choice) = chosen {
        if choice == 0 || choice > decision.options.len() {
            return Ok(Response::Error {
                message: format!(
                    "choice {} out of range (1..{})",
                    choice,
                    decision.options.len()
                ),
            });
        }
    }

    // Validate: at least one of chosen or message must be provided
    if chosen.is_none() && message.is_none() {
        return Ok(Response::Error {
            message: "must provide either a choice or a message (-m)".to_string(),
        });
    }

    let full_id = decision.id.as_str().to_string();
    let job_id = decision.job_id.clone();
    drop(state_guard);

    let resolved_at_ms = epoch_ms_now();

    // Emit DecisionResolved
    let event = Event::DecisionResolved {
        id: full_id.clone(),
        chosen,
        message: message.clone(),
        resolved_at_ms,
        namespace: String::new(), // filled from decision if needed
    };
    event_bus.send(event).map_err(|_| ConnectionError::WalError)?;

    // Emit JobResume to advance the job out of Waiting
    let resume_message = build_resume_message(chosen, message.as_deref(), &full_id);
    let resume_event = Event::JobResume {
        id: JobId::new(job_id),
        message: Some(resume_message),
        vars: HashMap::new(),
    };
    event_bus.send(resume_event).map_err(|_| ConnectionError::WalError)?;

    Ok(Response::DecisionResolved { id: full_id })
}
```

**4d. Wire up in `crates/daemon/src/listener/mod.rs`:**

Add `mod decisions;` and handle `Request::DecisionResolve` in `handle_request()`.

### Phase 5: CLI Commands (`crates/cli`)

**Goal:** Implement `oj decision` subcommand and `oj decisions` shorthand.

**5a. New `crates/cli/src/commands/decision.rs`:**

```rust
#[derive(Args)]
pub struct DecisionArgs {
    #[command(subcommand)]
    pub command: DecisionCommand,
}

#[derive(Subcommand)]
pub enum DecisionCommand {
    /// List pending decisions
    List {
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// Show details of a decision
    Show {
        /// Decision ID (or prefix)
        id: String,
    },
    /// Resolve a pending decision
    Resolve {
        /// Decision ID (or prefix)
        id: String,
        /// Pick a numbered option (1-indexed)
        choice: Option<usize>,
        /// Freeform message or answer
        #[arg(short = 'm', long)]
        message: Option<String>,
    },
}
```

Handler function pattern follows `queue.rs`:
- `List` → `Query::ListDecisions` → print table: `ID  JOB  AGE  SOURCE  SUMMARY`
- `Show` → `Query::GetDecision` → print full context + numbered options
- `Resolve` → `Request::DecisionResolve` → print confirmation

For `Show` output:
```
Decision: abc12345
Job: my-job (def67890)
Source:   gate
Age:      5m ago

Context:
  Gate command `./check.sh` failed with exit code 1.
  stderr: "validation failed: missing field 'name'"

Options:
  1. Retry       (recommended)
  2. Skip
  3. Cancel job

Use: oj decision resolve abc12345 <number> [-m message]
```

**5b. Update `crates/cli/src/commands/mod.rs`:**

```rust
pub mod decision;
```

**5c. Update `crates/cli/src/main.rs`:**

Add to `Commands` enum:
```rust
/// Decision management
Decision(decision::DecisionArgs),
/// List pending decisions (shorthand for `oj decision list`)
Decisions {
    #[arg(long = "project")]
    project: Option<String>,
},
```

Add dispatch in `run()`:
```rust
Commands::Decision(args) => {
    use decision::DecisionCommand;
    match &args.command {
        DecisionCommand::Resolve { .. } => {
            let client = DaemonClient::for_action()?;
            decision::handle(args.command, &client, &namespace, format).await?
        }
        DecisionCommand::List { .. } | DecisionCommand::Show { .. } => {
            let client = DaemonClient::for_query()?;
            decision::handle(args.command, &client, &namespace, format).await?
        }
    }
}
Commands::Decisions { project } => {
    let client = DaemonClient::for_query()?;
    decision::handle(
        decision::DecisionCommand::List { project },
        &client,
        &namespace,
        format,
    ).await?
}
```

### Phase 6: Tests & Fixup

**Goal:** Ensure everything compiles, passes tests, and handles edge cases.

**6a. Fix all `StepStatus::Waiting` usages:**

Every file that references `StepStatus::Waiting` (see list above) must be updated:
- `StepStatus::Waiting` → `StepStatus::Waiting(None)` for bare waiting (no decision)
- `== StepStatus::Waiting` → `.is_waiting()` for comparisons
- Pattern matches: `StepStatus::Waiting` → `StepStatus::Waiting(_)` or `StepStatus::Waiting(Some(ref decision_id))`

Key files:
- `crates/storage/src/state.rs:363` — `StepWaiting` handler: pass `None` or extract decision_id from event
- `crates/engine/src/runtime/monitor.rs:652` — use `is_waiting()`
- `crates/daemon/src/lifecycle.rs:626` — use `is_waiting()`
- `crates/daemon/src/listener/query.rs:606` — use `StepStatus::Waiting(_)` pattern
- Test files in `crates/engine/src/runtime_tests/` and `crates/storage/src/state_tests/`

**6b. Unit tests for core types (`crates/core/src/decision_tests.rs`):**
- Serde round-trip for `Decision`, `DecisionSource`, `DecisionOption`
- `is_resolved()` returns correct values

**6c. State tests (`crates/storage/src/state_tests/mod.rs`):**
- `DecisionCreated` event creates decision in state and sets job to `Waiting(Some(id))`
- `DecisionResolved` event updates chosen/message/resolved_at
- Idempotency: duplicate `DecisionCreated` is a no-op
- Prefix lookup via `get_decision()`

**6d. Event serde tests (`crates/core/src/event_tests.rs`):**
- Round-trip serialization of `DecisionCreated` and `DecisionResolved`
- Backward compat: `StepStatus::Waiting` deserializes from old format (no data)

**6e. CLI tests (`crates/cli/src/commands/decision_tests.rs`):**
- Verify `DecisionCommand` parsing (clap derives)

## Key Implementation Details

### Decision ID Format
Use `uuid::Uuid::new_v4().to_string()` — same as job IDs and queue item IDs. Prefix matching supported in `get_decision()`.

### StepStatus Backward Compatibility
The `StepStatus::Waiting` change removes `Copy` from the enum. This is a compile-time-visible breaking change — all affected call sites will fail to compile until updated. Use `#[serde(deserialize_with = ...)]` or `#[serde(alias = ...)]` if needed for WAL replay of old snapshots. Alternatively, since `Waiting` is serialized as `"Waiting"` (unit variant), add a custom deserializer that maps both `"Waiting"` (old) and `{"Waiting": null}` (new with None) to `StepStatus::Waiting(None)`.

Specifically, since `StepStatus` currently serializes as:
```json
"Waiting"
```
and with the new variant it would serialize as:
```json
{"Waiting": null}
```
A custom serde impl or `#[serde(untagged)]` approach is needed. The simplest approach: use a string-based serialization with a custom impl that handles both forms during deserialization. Alternatively, add `#[serde(alias)]` handling. The decision_id in `Waiting` is primarily in-memory state derived from events during replay, so the serialization format in snapshots can be kept simple.

### DecisionResolved → JobResume Coupling
When a decision is resolved, the daemon handler emits both `DecisionResolved` and `JobResume` events. This is intentional: the decision records the human's answer, and the job resume tells the engine to continue. This two-event approach means decisions can be resolved without necessarily resuming the job (future flexibility), and job resumes can happen without decisions (backward compat with `oj job resume`).

### StepWaiting Event — decision_id Field
Add an optional `decision_id` field to the existing `StepWaiting` event:

```rust
#[serde(rename = "step:waiting")]
StepWaiting {
    job_id: JobId,
    step: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decision_id: Option<String>,
},
```

The `apply_event` handler for `StepWaiting` passes `decision_id` into `StepStatus::Waiting(decision_id)`.

### Age Display
Use the existing `crate::output::format_time_ago()` helper for age column in list output, converting from `created_at_ms`.

## Verification Plan

1. **Compile check:** `cargo build --all` — verifies all `StepStatus::Waiting` migration sites compile
2. **Unit tests:** `cargo test -p oj-core` — decision type serde, StepStatus serde compat
3. **Storage tests:** `cargo test -p oj-storage` — event application, idempotency, prefix lookup
4. **Full test suite:** `cargo test --all` — catches any broken engine/daemon tests from StepStatus change
5. **Clippy:** `cargo clippy --all -- -D warnings`
6. **Format:** `cargo fmt --all`
7. **Manual smoke test:**
   - Start daemon, create a job that escalates
   - `oj decisions` — see the pending decision
   - `oj decision show <id>` — see full context
   - `oj decision resolve <id> 1 -m "looks good"` — resolve it
   - Verify job resumes
8. **Full CI:** `make check` (includes deny, quench)
