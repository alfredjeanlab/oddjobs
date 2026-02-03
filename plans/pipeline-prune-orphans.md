# Plan: `--orphans` flag for `oj pipeline prune`

## Overview

Add an `--orphans` flag to `oj pipeline prune` that detects and prunes orphaned pipelines — pipelines whose breadcrumb files exist on disk but have no corresponding active pipeline in daemon state. This leverages the existing orphan registry (`Arc<Mutex<Vec<Breadcrumb>>>`) populated at daemon startup and the existing `handle_dismiss_orphan` pattern for cleanup.

## Project Structure

```
crates/
├── cli/src/commands/pipeline.rs   # Add --orphans CLI arg, pass to client
├── cli/src/client.rs              # Add `orphans` param to pipeline_prune()
├── daemon/src/protocol.rs         # Add `orphans: bool` to PipelinePrune request
├── daemon/src/protocol_tests.rs   # Backward-compat deserialization test
├── daemon/src/listener/mod.rs     # Pass orphans + orphans registry to handler
└── daemon/src/listener/mutations.rs  # Update handle_pipeline_prune to prune orphans
```

## Dependencies

No new external dependencies. Uses existing:
- `oj_engine::breadcrumb::Breadcrumb` and `oj_engine::log_paths::breadcrumb_path()`
- `parking_lot::Mutex` (already in scope in mutations.rs via listener)
- Orphan registry `Arc<Mutex<Vec<Breadcrumb>>>` (already available in listener dispatch)

## Implementation Phases

### Phase 1: Protocol — Add `orphans` field to `PipelinePrune` request

**File:** `crates/daemon/src/protocol.rs`

Add `orphans: bool` with `#[serde(default)]` to the `PipelinePrune` variant for backward compatibility (same pattern as `failed`):

```rust
PipelinePrune {
    all: bool,
    #[serde(default)]
    failed: bool,
    #[serde(default)]
    orphans: bool,
    dry_run: bool,
},
```

**File:** `crates/daemon/src/protocol_tests.rs`

Add a roundtrip test for the new field and verify backward compat (old JSON without `orphans` deserializes to `false`).

**Verification:** `cargo check -p oj-daemon`

### Phase 2: Daemon — Update `handle_pipeline_prune` to prune orphans

**File:** `crates/daemon/src/listener/mutations.rs`

Update the function signature to accept the orphan registry and the `orphans` flag:

```rust
pub(super) fn handle_pipeline_prune(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    logs_path: &std::path::Path,
    orphans_registry: &Arc<Mutex<Vec<oj_engine::breadcrumb::Breadcrumb>>>,
    all: bool,
    failed: bool,
    orphans: bool,
    dry_run: bool,
) -> Result<Response, ConnectionError> {
```

When `orphans` is `true`:
1. Lock the orphan registry and collect all orphan entries as `PipelineEntry` items (with `step` set to `"orphaned"`).
2. Add them to `to_prune`.
3. On non-dry-run: remove each from the orphan registry and delete breadcrumb files, pipeline logs, and agent logs/dirs (same cleanup as existing prune).

When `orphans` is `false`, behavior is unchanged.

Key detail: when `--orphans` is used alone (without `--all` or `--failed`), it should **only** prune orphans — skip the normal terminal-pipeline loop. When combined with `--all` or `--failed`, it prunes both terminal pipelines (per existing logic) and orphans.

```rust
// When --orphans flag is set, collect orphaned pipelines
if orphans {
    let mut orphan_guard = orphans_registry.lock();
    let drain_indices: Vec<usize> = (0..orphan_guard.len()).collect();
    for &i in drain_indices.iter().rev() {
        let bc = &orphan_guard[i];
        to_prune.push(PipelineEntry {
            id: bc.pipeline_id.clone(),
            name: bc.name.clone(),
            step: "orphaned".to_string(),
        });
        if !dry_run {
            let removed = orphan_guard.remove(i);
            // Delete breadcrumb file
            let crumb = oj_engine::log_paths::breadcrumb_path(logs_path, &removed.pipeline_id);
            let _ = std::fs::remove_file(&crumb);
            // Delete pipeline log
            let log_file = logs_path.join(format!("{}.log", removed.pipeline_id));
            let _ = std::fs::remove_file(&log_file);
            // Delete agent logs/dirs
            let agent_log = logs_path.join("agent").join(format!("{}.log", removed.pipeline_id));
            let _ = std::fs::remove_file(&agent_log);
            let agent_dir = logs_path.join("agent").join(&removed.pipeline_id);
            let _ = std::fs::remove_dir_all(&agent_dir);
        }
    }
}
```

**File:** `crates/daemon/src/listener/mod.rs`

Update the `PipelinePrune` dispatch to pass the orphan registry and `orphans` flag:

```rust
Request::PipelinePrune {
    all,
    failed,
    orphans,
    dry_run,
} => mutations::handle_pipeline_prune(
    state, event_bus, logs_path, &self.orphans, // or however the registry is accessed
    all, failed, orphans, dry_run,
),
```

Note: Check how the orphan registry is accessed in the listener. It's already used for query dispatch (e.g., `handle_list_orphans`), so the same `Arc<Mutex<Vec<Breadcrumb>>>` reference should be passed through.

**Verification:** `cargo check -p oj-daemon`

### Phase 3: CLI — Add `--orphans` flag and wire through client

**File:** `crates/cli/src/commands/pipeline.rs`

Add the flag to the `Prune` variant:

```rust
Prune {
    #[arg(long)]
    all: bool,
    #[arg(long)]
    failed: bool,
    /// Prune orphaned pipelines (breadcrumb exists but no daemon state)
    #[arg(long)]
    orphans: bool,
    #[arg(long)]
    dry_run: bool,
},
```

Update the handler to pass `orphans` to the client:

```rust
PipelineCommand::Prune {
    all,
    failed,
    orphans,
    dry_run,
} => {
    let (pruned, skipped) = client.pipeline_prune(all, failed, orphans, dry_run).await?;
    // ... rest unchanged ...
}
```

**File:** `crates/cli/src/client.rs`

Update `pipeline_prune` to accept and forward `orphans`:

```rust
pub async fn pipeline_prune(
    &self,
    all: bool,
    failed: bool,
    orphans: bool,
    dry_run: bool,
) -> Result<(Vec<oj_daemon::PipelineEntry>, usize), ClientError> {
    let req = Request::PipelinePrune {
        all,
        failed,
        orphans,
        dry_run,
    };
    match self.send(&req).await? {
        Response::PipelinesPruned { pruned, skipped } => Ok((pruned, skipped)),
        other => Self::reject(other),
    }
}
```

**Verification:** `cargo check -p oj-cli`

### Phase 4: Tests and full verification

1. Add a protocol roundtrip test in `protocol_tests.rs` for `PipelinePrune` with `orphans: true`.
2. Add a backward-compat test: JSON without `orphans` field deserializes with `orphans: false`.
3. Run `make check` for full verification (fmt, clippy, tests, build, audit).

**Verification:** `make check`

## Key Implementation Details

1. **Exclusive vs. combined mode:** When `--orphans` is the only flag, skip the normal terminal-pipeline iteration entirely (the `skipped` count should not include active pipelines). When combined with `--all` or `--failed`, run both the terminal-pipeline loop and the orphan collection.

2. **Lock ordering:** The orphan registry lock should be acquired and released before or after (not concurrently with) the `MaterializedState` lock to avoid potential deadlocks. Since orphan pruning is independent of state pruning, acquire state lock first (existing code), release it, then acquire orphan lock.

3. **Output format:** Orphaned pipelines appear in the same output list as terminal pipelines, with `step: "orphaned"`. The CLI output line reads e.g. `Pruned my-pipeline (a1b2c3d4e5f6, orphaned)`.

4. **`--all` does NOT imply `--orphans`:** The `--all` flag means "all terminal pipelines regardless of age" — it does not automatically include orphans. Orphans are a separate category that requires explicit opt-in via `--orphans`.

5. **Workspace cleanup:** Orphaned pipelines may have associated ephemeral workspaces. The breadcrumb stores `workspace_root`. Consider whether to also clean up the workspace directory. For this initial implementation, only clean up logs and breadcrumb files (consistent with existing prune behavior — workspace cleanup is a separate `oj workspace prune` concern).

## Verification Plan

1. **Unit tests:**
   - Protocol roundtrip with `orphans: true`
   - Backward-compat deserialization (missing `orphans` field → `false`)

2. **Manual testing:**
   - Start daemon, run a pipeline, kill daemon mid-execution, restart → orphan detected
   - `oj pipeline prune --orphans --dry-run` → shows orphaned pipeline(s)
   - `oj pipeline prune --orphans` → removes orphaned pipeline(s), breadcrumb files deleted
   - `oj pipeline prune --all` → does NOT remove orphans (only terminal)
   - `oj pipeline prune --all --orphans` → removes both terminal and orphans
   - `oj pipeline list` → orphans no longer appear after prune

3. **Full suite:** `make check` passes (fmt, clippy, all tests, build, audit, deny).
