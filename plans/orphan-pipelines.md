# Orphan Pipelines in Pipeline List/Show/Status

## Overview

Surface orphaned pipelines (detected via breadcrumb files at daemon startup) in `oj pipeline list`, `oj pipeline show`, and `oj status`. Currently orphans are stored in an internal `Arc<Mutex<Vec<Breadcrumb>>>` and only visible through `oj daemon orphans`. This plan integrates them into the standard pipeline commands so users discover orphans naturally, can inspect them, and take action (attach to tmux, recover, or dismiss).

## Project Structure

Files to modify:

```
crates/daemon/src/listener/query.rs       # Include orphans in ListPipelines, GetPipeline, StatusOverview
crates/daemon/src/protocol_status.rs      # Add orphaned_pipelines field to NamespaceStatus
crates/cli/src/commands/pipeline.rs       # Handle "Orphaned" status in list/show display
crates/cli/src/commands/status.rs         # Render orphaned pipelines section
```

## Dependencies

No new external dependencies required. All changes use existing types (`Breadcrumb`, `OrphanSummary`, `PipelineSummary`, `PipelineDetail`, `NamespaceStatus`).

## Implementation Phases

### Phase 1: Include orphans in `ListPipelines` response

**Goal:** `oj pipeline list` shows orphaned pipelines with status "Orphaned".

**File:** `crates/daemon/src/listener/query.rs` — `Query::ListPipelines` handler (lines 46-69)

Currently the handler only iterates `state.pipelines`. After building the `pipelines` vec from state, append orphan entries:

```rust
Query::ListPipelines => {
    let pipelines = state
        .pipelines
        .values()
        .map(|p| { /* existing mapping */ })
        .collect::<Vec<_>>();

    // Append orphaned pipelines from breadcrumb data
    let orphan_entries: Vec<PipelineSummary> = {
        let orphans = orphans.lock();
        orphans.iter().map(|bc| {
            let updated_at_ms = chrono::DateTime::parse_from_rfc3339(&bc.updated_at)
                .map(|dt| dt.timestamp_millis() as u64)
                .unwrap_or(0);
            PipelineSummary {
                id: bc.pipeline_id.clone(),
                name: bc.name.clone(),
                kind: bc.kind.clone(),
                step: bc.current_step.clone(),
                step_status: "Orphaned".to_string(),
                created_at_ms: updated_at_ms, // best available
                updated_at_ms,
                namespace: bc.project.clone(),
            }
        }).collect()
    };

    let mut pipelines = pipelines;
    pipelines.extend(orphan_entries);
    Response::Pipelines { pipelines }
}
```

**Key detail:** The `Query::ListPipelines` handler currently takes `state` lock but orphans are handled before the lock (lines 35-41). Move the orphans access into the `ListPipelines` arm. The orphans `Arc<Mutex<>>` is already passed to `handle_query`.

**Timestamp parsing:** Breadcrumb `updated_at` is ISO 8601 (RFC 3339) string. Parse with `chrono::DateTime::parse_from_rfc3339`. The `chrono` crate is already a dependency of the daemon (check `Cargo.toml`; if not, use a manual parse or add it).

**Verification:** Run `oj pipeline list` — orphaned pipelines appear with "Orphaned" status. Run `oj pipeline list --status orphaned` — filters to only orphans.

### Phase 2: Include orphans in `GetPipeline` response

**Goal:** `oj pipeline show <id>` works for orphaned pipeline IDs.

**File:** `crates/daemon/src/listener/query.rs` — `Query::GetPipeline` handler (lines 72-121)

Currently looks up `state.get_pipeline(&id)`. If `None`, fall back to searching orphans by ID/prefix:

```rust
Query::GetPipeline { id } => {
    let pipeline = state.get_pipeline(&id).map(|p| {
        /* existing PipelineDetail construction */
    });

    // If not found in state, check orphans
    let pipeline = pipeline.or_else(|| {
        let orphans = orphans.lock();
        orphans.iter()
            .find(|bc| bc.pipeline_id == id || bc.pipeline_id.starts_with(&id))
            .map(|bc| {
                Box::new(PipelineDetail {
                    id: bc.pipeline_id.clone(),
                    name: bc.name.clone(),
                    kind: bc.kind.clone(),
                    step: bc.current_step.clone(),
                    step_status: "Orphaned".to_string(),
                    vars: bc.vars.clone(),
                    workspace_path: bc.workspace_root.clone(),
                    session_id: bc.agents.first()
                        .and_then(|a| a.session_name.clone()),
                    error: Some("Pipeline was not recovered from WAL/snapshot".to_string()),
                    steps: Vec::new(),
                    agents: bc.agents.iter().map(|a| {
                        AgentSummary {
                            pipeline_id: bc.pipeline_id.clone(),
                            step_name: bc.current_step.clone(),
                            agent_id: a.agent_id.clone(),
                            agent_name: None,
                            namespace: Some(bc.project.clone()),
                            status: "orphaned".to_string(),
                            files_read: 0,
                            files_written: 0,
                            commands_run: 0,
                            exit_reason: None,
                            updated_at_ms: 0,
                        }
                    }).collect(),
                    namespace: bc.project.clone(),
                })
            })
    });

    Response::Pipeline { pipeline }
}
```

**Key detail:** The `session_id` from the first agent's `session_name` enables `oj pipeline attach <id>` to work for orphans, which is a core user goal.

**Verification:** `oj pipeline show <orphan-id>` displays orphan details including workspace path, session ID, and agents.

### Phase 3: Include orphans in `StatusOverview` / `NamespaceStatus`

**Goal:** `oj status` shows orphaned pipelines per namespace.

#### 3a: Add field to `NamespaceStatus`

**File:** `crates/daemon/src/protocol_status.rs` (lines 14-26)

Add a new field:

```rust
pub struct NamespaceStatus {
    pub namespace: String,
    pub active_pipelines: Vec<PipelineStatusEntry>,
    pub escalated_pipelines: Vec<PipelineStatusEntry>,
    pub orphaned_pipelines: Vec<PipelineStatusEntry>,  // NEW
    pub workers: Vec<WorkerSummary>,
    pub queues: Vec<QueueStatus>,
    pub active_agents: Vec<AgentStatusEntry>,
}
```

#### 3b: Populate in `StatusOverview` handler

**File:** `crates/daemon/src/listener/query.rs` — `Query::StatusOverview` handler (lines 457-614)

After the existing pipeline loop, add orphan collection:

```rust
// Collect orphaned pipelines grouped by namespace
let mut ns_orphaned: BTreeMap<String, Vec<PipelineStatusEntry>> = BTreeMap::new();
{
    let orphans = orphans.lock();
    for bc in orphans.iter() {
        let updated_at_ms = chrono::DateTime::parse_from_rfc3339(&bc.updated_at)
            .map(|dt| dt.timestamp_millis() as u64)
            .unwrap_or(0);
        let elapsed_ms = now_ms.saturating_sub(updated_at_ms);
        ns_orphaned.entry(bc.project.clone()).or_default().push(PipelineStatusEntry {
            id: bc.pipeline_id.clone(),
            name: bc.name.clone(),
            kind: bc.kind.clone(),
            step: bc.current_step.clone(),
            step_status: "Orphaned".to_string(),
            elapsed_ms,
            waiting_reason: None,
        });
    }
}
```

Add `ns_orphaned` keys to `all_namespaces` set. Include in `NamespaceStatus` construction:

```rust
NamespaceStatus {
    orphaned_pipelines: ns_orphaned.remove(&ns).unwrap_or_default(),
    // ... existing fields
}
```

#### 3c: Render in `oj status` CLI

**File:** `crates/cli/src/commands/status.rs` — `format_text` function (lines 59-199)

Add orphaned pipelines to header counts and namespace sections:

```rust
// In header line
let total_orphaned: usize = namespaces.iter().map(|ns| ns.orphaned_pipelines.len()).sum();
if total_orphaned > 0 {
    let _ = write!(out, " | {} orphaned", total_orphaned);
}

// In has_content check
let has_content = /* existing checks */ || !ns.orphaned_pipelines.is_empty();

// New section after escalated pipelines
if !ns.orphaned_pipelines.is_empty() {
    let _ = writeln!(out, "  Orphaned ({}):", ns.orphaned_pipelines.len());
    for p in &ns.orphaned_pipelines {
        let short_id = truncate_id(&p.id, 12);
        let elapsed = format_duration_ms(p.elapsed_ms);
        let _ = writeln!(
            out,
            "    ⚠ {}  {}  {}  Orphaned  {}",
            short_id, p.name, p.step, elapsed,
        );
    }
    let _ = writeln!(out, "    Run `oj daemon orphans` for recovery details");
    out.push('\n');
}
```

**Verification:** `oj status` shows orphaned pipelines in each namespace. Header shows orphan count.

### Phase 4: Handle "Orphaned" in pipeline list CLI filtering

**Goal:** The `--status orphaned` filter and display work correctly.

**File:** `crates/cli/src/commands/pipeline.rs`

The existing filter (line 268-273) already does case-insensitive comparison:
```rust
pipelines.retain(|p| {
    p.step_status.to_lowercase() == st_lower || p.step.to_lowercase() == st_lower
});
```

Since we set `step_status: "Orphaned"`, `--status orphaned` already works. No code change needed here.

For `format_pipeline_list`, no changes needed — orphans will display with their "Orphaned" status in the STATUS column naturally.

**Verification:** `oj pipeline list --status orphaned` shows only orphans. `oj pipeline list` shows all pipelines including orphans.

### Phase 5: Unit tests

**Goal:** Verify orphan integration in query handlers.

**File:** `crates/daemon/src/listener/query_tests.rs` (or appropriate test file)

Tests to add:

1. **`test_list_pipelines_includes_orphans`** — Create a breadcrumb in the orphans vec, call `handle_query(Query::ListPipelines, ...)`, verify response includes an entry with `step_status: "Orphaned"`.

2. **`test_get_pipeline_falls_back_to_orphan`** — Populate orphans vec with a breadcrumb, call `handle_query(Query::GetPipeline { id })` with the orphan's ID, verify `PipelineDetail` is returned with orphan data.

3. **`test_get_pipeline_prefers_state_over_orphan`** — Put same ID in both state and orphans, verify state version is returned (not orphan).

4. **`test_status_overview_includes_orphans`** — Populate orphans, call `StatusOverview`, verify `NamespaceStatus.orphaned_pipelines` is populated.

5. **`test_list_pipelines_orphan_prefix_match`** — Verify `GetPipeline` prefix matching works for orphan IDs.

## Key Implementation Details

### Timestamp conversion

Breadcrumb stores `updated_at` as an RFC 3339 string (e.g., `"2026-01-15T10:30:00Z"`). The `PipelineSummary` and `PipelineStatusEntry` use `u64` milliseconds since epoch. Use `chrono::DateTime::parse_from_rfc3339` for conversion. If `chrono` is not already a dependency of the daemon crate, either add it or use a simple manual parse (split on `T`, etc.). Check `crates/daemon/Cargo.toml` first.

### Lock ordering

The `handle_query` function takes `state: &Arc<Mutex<MaterializedState>>` and `orphans: &Arc<Mutex<Vec<Breadcrumb>>>`. Currently `ListOrphans` and `DismissOrphan` are handled before the state lock (lines 35-41). For `ListPipelines` and `GetPipeline`, acquire the state lock first (existing behavior), then acquire the orphans lock briefly inside. This avoids holding both locks simultaneously for most of the handler. For `StatusOverview`, acquire orphans lock in a separate block after the state lock is released, or acquire orphans lock briefly while state lock is held (the orphans lock is fast — just reads a vec).

### No WAL changes

Orphans are not written to the WAL or stored in `MaterializedState`. They remain in the in-memory `Vec<Breadcrumb>` populated at startup. This is intentional — orphans represent state that *failed* to persist, so storing them in the WAL would be circular.

### Deduplication

Since orphans are detected at startup by cross-referencing breadcrumbs against recovered state, there should be no duplicates between `state.pipelines` and the orphans vec. The phase 2 fallback (`or_else`) ensures state pipelines take priority.

## Verification Plan

1. **Unit tests** (Phase 5) — verify query handler behavior with orphans
2. **Manual test flow:**
   - Start daemon, create a pipeline, kill daemon mid-pipeline (simulate crash)
   - Delete the WAL/snapshot files but leave breadcrumb `.crumb.json` files
   - Restart daemon — orphan should be detected
   - `oj pipeline list` — verify orphan appears with "Orphaned" status
   - `oj pipeline list --status orphaned` — verify filter works
   - `oj pipeline show <orphan-id>` — verify details shown (workspace, session, agents)
   - `oj status` — verify orphan appears in namespace section
   - `oj daemon orphans` — verify existing command still works
   - `oj daemon dismiss-orphan <id>` — verify orphan disappears from all views
3. **`make check`** — full CI verification (fmt, clippy, tests, build, audit, deny)
