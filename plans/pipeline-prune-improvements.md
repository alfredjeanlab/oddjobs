# Pipeline Prune Improvements

## Overview

Improve `oj pipeline prune` with two changes:
1. Add a `--failed` flag that prunes all failed pipelines regardless of age.
2. In the default prune logic (without `--failed` or `--all`), always prune cancelled pipelines regardless of age — currently they respect the 12-hour age threshold like other terminal states.

## Project Structure

Files to modify:

```
crates/
├── cli/src/commands/pipeline.rs   # Add --failed CLI arg, pass to client
├── cli/src/client.rs              # Add `failed` param to pipeline_prune()
├── daemon/src/protocol.rs         # Add `failed: bool` to PipelinePrune request
├── daemon/src/listener/mutations.rs  # Update handle_pipeline_prune logic
└── daemon/src/listener.rs         # Pass `failed` through dispatch (if needed)
```

## Dependencies

No new external dependencies required.

## Implementation Phases

### Phase 1: Protocol — Add `failed` field to `PipelinePrune` request

**File:** `crates/daemon/src/protocol.rs`

Add `failed: bool` to the `PipelinePrune` variant:

```rust
PipelinePrune {
    /// Prune all terminal pipelines regardless of age
    all: bool,
    /// Prune all failed pipelines regardless of age
    failed: bool,
    /// Preview only -- don't actually delete
    dry_run: bool,
},
```

**Verification:** `cargo check -p oj-daemon` — will show compile errors in the handler and listener dispatch, confirming the field propagates.

### Phase 2: Daemon — Update `handle_pipeline_prune` logic

**File:** `crates/daemon/src/listener/mutations.rs` (function `handle_pipeline_prune`)

Add the `failed: bool` parameter to the function signature. Update the filtering logic inside the loop:

```rust
pub(super) fn handle_pipeline_prune(
    state: &Arc<Mutex<MaterializedState>>,
    event_bus: &EventBus,
    logs_path: &std::path::Path,
    all: bool,
    failed: bool,
    dry_run: bool,
) -> Result<Response, ConnectionError> {
    // ... existing setup ...

    for pipeline in state_guard.pipelines.values() {
        if !pipeline.is_terminal() {
            skipped += 1;
            continue;
        }

        // Determine if this pipeline skips the age check:
        // - --all: everything skips age check
        // - --failed: failed pipelines skip age check
        // - cancelled pipelines always skip age check (default behavior)
        let skip_age_check = all
            || (failed && pipeline.step == "failed")
            || pipeline.step == "cancelled";

        if !skip_age_check {
            let created_at_ms = pipeline
                .step_history
                .first()
                .map(|r| r.started_at_ms)
                .unwrap_or(0);
            if created_at_ms > 0 && now_ms.saturating_sub(created_at_ms) < age_threshold_ms {
                skipped += 1;
                continue;
            }
        }

        // --failed flag: only prune failed pipelines (skip done/cancelled)
        if failed && pipeline.step != "failed" {
            skipped += 1;
            continue;
        }

        to_prune.push(PipelineEntry { /* ... */ });
    }
    // ... rest unchanged ...
}
```

Key logic:
- When `--failed` is set: prune **only** failed pipelines, regardless of age.
- In the default path (no flags): cancelled pipelines bypass the age threshold; done and failed pipelines still respect it.
- `--all` is unchanged: prunes all terminal pipelines regardless of age.

Also update the listener dispatch site (likely in `crates/daemon/src/listener.rs` or nearby) to pass the new `failed` field through.

**Verification:** `cargo check -p oj-daemon`

### Phase 3: CLI & Client — Add `--failed` flag

**File:** `crates/cli/src/commands/pipeline.rs`

Add the `--failed` arg to the `Prune` variant:

```rust
Prune {
    /// Remove all terminal pipelines regardless of age
    #[arg(long)]
    all: bool,
    /// Remove all failed pipelines regardless of age
    #[arg(long)]
    failed: bool,
    /// Show what would be pruned without doing it
    #[arg(long)]
    dry_run: bool,
},
```

Update the match arm to destructure and pass `failed`:

```rust
PipelineCommand::Prune { all, failed, dry_run } => {
    let (pruned, skipped) = client.pipeline_prune(all, failed, dry_run).await?;
    // ... rest unchanged ...
}
```

**File:** `crates/cli/src/client.rs`

Update `pipeline_prune` to accept and forward the `failed` param:

```rust
pub async fn pipeline_prune(
    &self,
    all: bool,
    failed: bool,
    dry_run: bool,
) -> Result<(Vec<oj_daemon::PipelineEntry>, usize), ClientError> {
    let request = Request::PipelinePrune { all, failed, dry_run };
    // ... rest unchanged ...
}
```

**Verification:** `cargo check -p oj-cli`

### Phase 4: Tests & Final Verification

1. Add a protocol round-trip test for the new `failed` field in `crates/daemon/src/protocol_tests.rs`.
2. Run `make check` to ensure everything compiles, passes clippy, and all existing tests pass.

## Key Implementation Details

- **`--failed` is exclusive in scope:** When `--failed` is passed, only failed pipelines are pruned (not done or cancelled). This makes it a targeted cleanup tool.
- **Cancelled always prune:** In the default path, cancelled pipelines are treated like stale artifacts — they are always prunable regardless of age. This reflects that cancellation is a user action, not a result worth preserving for review.
- **`--all` unchanged:** The `--all` flag continues to prune all terminal pipelines regardless of age, providing the "nuclear option."
- **Flag interaction:** `--all --failed` would behave like `--failed` (the failed filter takes priority since it narrows scope). If this is undesirable, we could make them mutually exclusive via clap's `conflicts_with`, but keeping them composable is simpler.

## Verification Plan

1. **Unit test:** Protocol encode/decode round-trip with `failed: true`.
2. **Manual test:** Run `oj pipeline prune --dry-run` and verify cancelled pipelines appear in output even when recent.
3. **Manual test:** Run `oj pipeline prune --failed --dry-run` and verify only failed pipelines appear.
4. **`make check`:** Full CI-equivalent verification (fmt, clippy, tests, build, audit, deny).
