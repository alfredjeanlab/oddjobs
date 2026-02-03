# Status Command

## Overview

Add `oj status` as a top-level CLI command that shows a quick overview dashboard of the daemon's current state. Displays active/escalated pipelines, queued items, active agents, running workers, and queue health — grouped by project namespace. This is the primary human-in-the-loop entry point for checking what's happening across all projects.

The command composes data from existing queries (pipelines, agents, workers, queue items) into a single dashboard view. A new `Query::StatusOverview` request fetches all data in one IPC round-trip to keep the implementation efficient.

## Project Structure

Files to create or modify:

```
crates/daemon/src/protocol.rs       # Add StatusOverview query + StatusOverview response
crates/daemon/src/listener/query.rs # Add StatusOverview handler
crates/cli/src/client.rs            # Add status_overview() client method
crates/cli/src/commands/mod.rs      # Add status module
crates/cli/src/commands/status.rs   # NEW: status command implementation + formatting
crates/cli/src/main.rs              # Add Commands::Status variant + dispatch
```

## Dependencies

No new external dependencies. Uses existing `clap`, `serde`, `serde_json`, `std::fmt::Write`.

## Implementation Phases

### Phase 1: Protocol — Add `StatusOverview` Query and Response

**Files:** `crates/daemon/src/protocol.rs`

1. Add `StatusOverview` variant to the `Query` enum (no parameters — returns all namespaces):

```rust
/// Get a cross-project status overview
StatusOverview,
```

2. Add `StatusOverview` variant to the `Response` enum:

```rust
/// Cross-project status overview
StatusOverview {
    uptime_secs: u64,
    namespaces: Vec<NamespaceStatus>,
},
```

3. Add the `NamespaceStatus` struct:

```rust
/// Per-namespace status summary
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceStatus {
    pub namespace: String,
    /// Non-terminal pipelines (Running/Pending status)
    pub active_pipelines: Vec<PipelineStatusEntry>,
    /// Pipelines in Waiting status (escalated to human)
    pub escalated_pipelines: Vec<PipelineStatusEntry>,
    /// Workers and their status
    pub workers: Vec<WorkerSummary>,
    /// Queue depths: (queue_name, pending_count, active_count, dead_count)
    pub queues: Vec<QueueStatus>,
    /// Currently running agents
    pub active_agents: Vec<AgentStatusEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineStatusEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub step: String,
    pub step_status: String,
    /// Duration since pipeline started (ms)
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueStatus {
    pub name: String,
    pub pending: usize,
    pub active: usize,
    pub dead: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentStatusEntry {
    pub agent_id: String,
    pub pipeline_name: String,
    pub step_name: String,
    pub status: String,
}
```

**Milestone:** Protocol compiles with new types.

### Phase 2: Daemon Handler — Build StatusOverview from MaterializedState

**Files:** `crates/daemon/src/listener/query.rs`

Add a `Query::StatusOverview` match arm in `handle_query()`. The handler:

1. Iterates `state.pipelines` to collect non-terminal pipelines, separating `Waiting` (escalated) from `Running`/`Pending`.
2. Groups pipelines by `namespace`.
3. Iterates `state.workers` to collect worker summaries, grouped by namespace (extracted from the scoped key `namespace/name`).
4. Iterates `state.queue_items` to compute per-queue counts (pending/active/dead), grouped by namespace.
5. Computes agent summaries using the existing `compute_agent_summaries` pattern for non-terminal pipelines only.
6. Assembles `NamespaceStatus` for each namespace seen across any entity.
7. Sorts namespaces alphabetically.

Key logic for elapsed time:

```rust
let now_ms = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_millis() as u64;
let created_at_ms = p.step_history.first().map(|r| r.started_at_ms).unwrap_or(0);
let elapsed_ms = now_ms.saturating_sub(created_at_ms);
```

Key logic for escalated vs active:

```rust
match p.step_status {
    StepStatus::Waiting => escalated_pipelines.push(entry),
    _ => active_pipelines.push(entry),  // Running, Pending
}
```

**Milestone:** `oj daemon status` still works; new query returns data (testable via JSON output).

### Phase 3: CLI Client Method

**Files:** `crates/cli/src/client.rs`

Add a `status_overview()` method:

```rust
pub async fn status_overview(&self) -> Result<(u64, Vec<oj_daemon::NamespaceStatus>), ClientError> {
    let query = Request::Query {
        query: Query::StatusOverview,
    };
    match self.send(&query).await? {
        Response::StatusOverview { uptime_secs, namespaces } => Ok((uptime_secs, namespaces)),
        Response::Error { message } => Err(ClientError::Rejected(message)),
        _ => Err(ClientError::UnexpectedResponse),
    }
}
```

**Milestone:** Client compiles with new method.

### Phase 4: CLI Command — `oj status`

**Files:** `crates/cli/src/commands/status.rs` (new), `crates/cli/src/commands/mod.rs`, `crates/cli/src/main.rs`

1. Add `pub mod status;` to `commands/mod.rs`.

2. Add `Commands::Status` variant to the enum in `main.rs`:

```rust
/// Show overview of active work across all projects
Status,
```

3. Add dispatch in the `match` block (query semantics, no args):

```rust
Commands::Status => {
    let client = DaemonClient::for_query()?;
    status::handle(&client, format).await?
}
```

4. Create `status.rs` with the `handle` function and text/JSON formatting.

#### Text Output Format

```
oj daemon: up 2h | 3 active pipelines | 1 escalated

── oddjobs ──────────────────────────────────────
  Pipelines (2 active):
    abc123  fix/login-bug   work    Running   5m
    def456  chore/deps      plan    Running   12m

  Escalated (1):
    ⚠ ghi789  feat/auth     test    Waiting   1h
      → gate check failed (exit 1)

  Workers:
    fix-worker    ● running  2/3 active
    chore-worker  ● running  1/3 active

  Queues:
    merge   3 pending, 1 active
    fix     0 pending, 0 active

  Agents (3 running):
    abc123/work   claude-abc   running
    def456/plan   claude-def   running
    ghi789/test   claude-ghi   waiting

── gastown ──────────────────────────────────────
  Pipelines (1 active):
    ...
```

Design decisions for the text formatter:
- Header line shows daemon uptime and global counts.
- One section per namespace, separated by ruled headers.
- Pipelines show truncated ID (12 chars), name, current step, status, and elapsed duration.
- Escalated pipelines are highlighted with `⚠` and show the waiting reason (from `StepOutcome::Waiting(reason)`).
- Workers show name, status indicator (● running / ○ stopped), and active/concurrency ratio.
- Queues show pending + active counts; only shown if non-empty or if workers reference them. Dead items get a warning: `2 dead`.
- Agents show pipeline/step, agent ID, and status.
- Sections with zero items are omitted entirely.
- If the daemon is not running, print `oj daemon: not running` and exit with code 0.

JSON output returns the raw `StatusOverview` response.

#### Graceful Degradation

When the daemon is not running (`DaemonClient::for_query()` returns `ConnectionRefused`), the command prints `oj daemon: not running` instead of erroring. This is important since `oj status` is the first thing users run.

```rust
pub async fn handle(client: &DaemonClient, format: OutputFormat) -> Result<()> {
    let (uptime_secs, namespaces) = match client.status_overview().await {
        Ok(data) => data,
        Err(ClientError::ConnectionRefused) => {
            println!("oj daemon: not running");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    // ...format and print...
}
```

**Milestone:** `oj status` displays the dashboard. JSON mode works with `oj -o json status`.

### Phase 5: Escalation Detail — Show Waiting Reason

**Files:** `crates/daemon/src/protocol.rs`, `crates/daemon/src/listener/query.rs`

Enrich `PipelineStatusEntry` with an optional `waiting_reason`:

```rust
pub struct PipelineStatusEntry {
    // ...existing fields...
    /// Reason pipeline is waiting (from StepOutcome::Waiting)
    pub waiting_reason: Option<String>,
}
```

In the query handler, extract the reason from the pipeline's current step record:

```rust
let waiting_reason = match &p.step_history.last().map(|r| &r.outcome) {
    Some(StepOutcome::Waiting(reason)) => Some(reason.clone()),
    _ => None,
};
```

This provides actionable context in the dashboard (e.g., "gate check failed (exit 1)") so the user knows what to investigate.

**Milestone:** Escalated pipelines show their waiting reason in text output.

## Key Implementation Details

### Single IPC Round-Trip

Rather than making 4+ separate queries (list_pipelines, list_agents, list_workers, list_queue_items × N), the `StatusOverview` query builds the entire response server-side from `MaterializedState` in one lock acquisition. This avoids inconsistency between multiple reads and reduces latency.

### Namespace Grouping

All entities carry a `namespace` field. For workers and queues stored with scoped keys (`namespace/name`), the namespace is extracted by splitting on `/`. The handler collects all unique namespaces across all entity types and builds a `NamespaceStatus` for each.

### Pipeline Duration

Duration is computed as `now - created_at_ms` (time since pipeline's first step started). This is the wall-clock elapsed time, which is most useful for understanding "how long has this been going."

### Terminal Pipeline Filtering

Only non-terminal pipelines appear in the status output. A pipeline is terminal if its step is `"done"`, `"failed"`, or `"cancelled"` (via `Pipeline::is_terminal()`). This keeps the dashboard focused on what needs attention.

### Agent Extraction

Active agents are extracted using the existing `compute_agent_summaries` pattern from `query.rs`, filtered to only include agents with status `"running"` or `"waiting"`. This reuses the JSONL log parsing that already powers `oj agent list`.

### Graceful When Daemon Down

Since `oj status` uses `DaemonClient::for_query()` (connect-only, no auto-start), it gracefully handles the daemon being stopped. The command catches `ConnectionRefused` and prints a friendly message instead of an error.

## Verification Plan

1. **Unit test** the query handler: construct a `MaterializedState` with multiple namespaces, pipelines in various states (active, escalated, terminal), workers, and queue items. Assert the `StatusOverview` response contains correct grouping and counts.

2. **Compilation check**: `make check` passes (fmt, clippy, tests, build, audit, deny).

3. **Manual smoke tests**:
   - `oj status` with daemon stopped → prints "not running"
   - `oj status` with no active work → prints uptime, empty
   - `oj status` with active pipelines across 2+ namespaces → shows grouped sections
   - `oj status` with an escalated pipeline → shows ⚠ marker and waiting reason
   - `oj -o json status` → outputs valid JSON matching the `StatusOverview` response structure

4. **Integration test**: Start daemon, run a pipeline, verify `oj status` includes it. Cancel the pipeline, verify it disappears from status.
