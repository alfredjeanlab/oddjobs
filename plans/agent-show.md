# Plan: `oj agent show <id>`

## Overview

Add an `oj agent show <id>` subcommand that displays detailed information for a single agent. The command resolves an agent by ID (or prefix) across all pipelines and displays its identity, status, pipeline context, activity metrics, timestamps, and error info. Supports `-o json` output. Follows the same detail-view pattern as `oj pipeline show` and `oj workspace show`.

## Project Structure

Files to create or modify:

```
crates/
├── cli/src/commands/agent.rs      # Add Show variant + handler
├── cli/src/client.rs              # Add get_agent() client method
├── daemon/src/protocol.rs         # Add GetAgent query, AgentDetail type, Agent response
├── daemon/src/listener/query.rs   # Add GetAgent query handler
```

## Dependencies

No new external dependencies. Uses existing `serde`, `serde_json`, `clap`, and `oj_engine::log_paths`.

## Implementation Phases

### Phase 1: Add `AgentDetail` protocol type and `GetAgent` query

**Files:** `crates/daemon/src/protocol.rs`

1. Add `AgentDetail` struct with richer fields than `AgentSummary`:

```rust
/// Detailed agent information for `oj agent show`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentDetail {
    pub agent_id: String,
    pub agent_name: Option<String>,
    pub pipeline_id: String,
    pub pipeline_name: String,
    pub step_name: String,
    pub namespace: Option<String>,
    pub status: String,
    pub workspace_path: Option<PathBuf>,
    pub session_id: Option<String>,
    pub files_read: usize,
    pub files_written: usize,
    pub commands_run: usize,
    pub exit_reason: Option<String>,
    pub error: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub updated_at_ms: u64,
}
```

2. Add `GetAgent` variant to `Query` enum:

```rust
/// Get detailed info for a single agent by ID (or prefix)
GetAgent { agent_id: String },
```

3. Add `Agent` variant to `Response` enum:

```rust
/// Single agent details
Agent { agent: Option<Box<AgentDetail>> },
```

**Milestone:** Protocol compiles with new types.

### Phase 2: Implement `GetAgent` query handler

**File:** `crates/daemon/src/listener/query.rs`

Add a `Query::GetAgent` match arm in `handle_query()`. The handler:

1. Iterates `state.pipelines.values()` searching for a matching agent by ID or prefix (same logic as `find_agent()` in `agent.rs` and `ListAgents` in `query.rs`).
2. When found, computes activity metrics using the existing `compute_agent_summaries` pattern (scan agent log file for `read:`, `wrote:`, `edited:`, `bash:` lines).
3. Enriches with pipeline-level context: `pipeline_name`, `workspace_path`, `session_id`, `namespace`, `error` (from step detail).
4. Extracts `started_at_ms` and `finished_at_ms` from the matching `StepRecord`.
5. Returns `Response::Agent { agent: Some(Box::new(detail)) }` or `Response::Agent { agent: None }` if not found.

Pattern reference: The `Query::GetPipeline` handler at `query.rs:82-134` and `compute_agent_summaries()` at `query.rs:611-685`.

**Milestone:** Daemon handles `GetAgent` query and returns correct data.

### Phase 3: Add `get_agent()` client method

**File:** `crates/cli/src/client.rs`

Add a method following the `get_pipeline` / `get_workspace` pattern:

```rust
pub async fn get_agent(&self, agent_id: &str) -> Result<Option<AgentDetail>, ClientError> {
    let request = Request::Query {
        query: Query::GetAgent { agent_id: agent_id.to_string() },
    };
    match self.send(&request).await? {
        Response::Agent { agent } => Ok(agent.map(|b| *b)),
        other => Self::reject(other),
    }
}
```

**Milestone:** CLI can fetch agent detail from daemon.

### Phase 4: Add `Show` subcommand and CLI handler

**File:** `crates/cli/src/commands/agent.rs`

1. Add `Show` variant to `AgentCommand` enum:

```rust
/// Show detailed info for a single agent
Show {
    /// Agent ID (or prefix)
    id: String,
},
```

2. Add handler in the `match command` block:

**Text output format** (follows `pipeline show` pattern):

```
Agent: <full_agent_id>
  Name: <agent_name or "-">
  Project: <namespace>            (omit if empty)
  Pipeline: <pipeline_id> (<pipeline_name>)
  Step: <step_name>
  Status: <status>

  Activity:
    Files read: <n>
    Files written: <n>
    Commands run: <n>

  Session: <session_id>           (omit if none)
  Workspace: <workspace_path>     (omit if none)
  Started: <started_at_ms as relative time>
  Updated: <updated_at_ms as relative time>
  Error: <error or exit_reason>   (omit if none)
```

**JSON output:** `serde_json::to_string_pretty(&agent)` (the full `AgentDetail` struct).

**Not found:** Print `"Agent not found: {id}"` and exit.

**Milestone:** `oj agent show <id>` works end-to-end with both text and JSON output.

### Phase 5: Add tests

**Files:** `crates/cli/src/commands/agent_tests.rs` (extend existing), `crates/daemon/src/listener/query_tests.rs` (extend existing)

1. **Query handler test:** Create a `MaterializedState` with a pipeline containing agent step records, call `handle_query(Query::GetAgent { .. })`, assert the returned `AgentDetail` has correct fields. Test prefix matching. Test not-found returns `None`.

2. **CLI integration test (optional):** If existing e2e patterns support it, test `oj agent show` output format. Otherwise, unit-test the formatting logic.

**Milestone:** Tests pass for new query and display logic.

## Key Implementation Details

### Agent ID resolution

Agents are identified by `{pipeline_id}-{step_name}` compound IDs. The `GetAgent` handler must support prefix matching (same as `find_agent()` in `agent.rs:338-355`). When multiple agents match a prefix, return the first match (most recently updated preferred, matching `ListAgents` sort order).

### Reusing `compute_agent_summaries` logic

The `GetAgent` handler needs the same log-scanning logic as `compute_agent_summaries()`. Rather than duplicating, the handler can call `compute_agent_summaries()` for the matching pipeline and filter to the target agent, then enrich with pipeline-level fields (`pipeline_name`, `workspace_path`, `session_id`, `error`).

### Timestamp formatting

Use the existing `format_time_ago()` helper from `crates/cli/src/commands/pipeline.rs` (or the equivalent in the `output` module) for human-readable relative timestamps in text mode. If no shared helper exists, add a minimal one in the agent command module.

### Protocol import path

The new `AgentDetail` type needs to be added to the `use crate::protocol::{ ... }` import in `query.rs` and re-exported from `oj_daemon` if the client needs it (follow the pattern of `PipelineDetail`, `WorkspaceDetail`).

## Verification Plan

1. **`cargo check --all`** — Confirms all new types and query variants compile.
2. **`cargo test --all`** — Runs new and existing tests.
3. **`cargo clippy --all-targets --all-features -- -D warnings`** — No lint warnings.
4. **Manual test:** Start daemon, run a pipeline with an agent, then:
   - `oj agent list` — Confirm agent appears.
   - `oj agent show <full-id>` — Confirm detail output.
   - `oj agent show <prefix>` — Confirm prefix resolution works.
   - `oj agent show <full-id> -o json` — Confirm JSON output.
   - `oj agent show nonexistent` — Confirm "not found" message.
5. **`make check`** — Full verification suite passes.
