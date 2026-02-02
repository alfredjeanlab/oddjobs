# Wait Streaming

Stream step-level progress during `oj pipeline wait` so users see real-time updates as pipeline steps transition, instead of blocking silently until completion.

## Overview

Currently `oj pipeline wait` polls pipeline state in a loop and only prints output when a pipeline reaches a terminal state. This plan adds incremental step progress output by diffing the `steps` (step_history) returned by `GetPipeline` on each poll cycle and printing transitions as they happen.

This uses the client-side polling approach (option a from the instructions) — no daemon or protocol changes needed. The `PipelineDetail.steps` field already contains full `StepRecordDetail` entries with names, outcomes, timestamps, and error details.

## Project Structure

All changes are in the CLI crate.

```
crates/cli/src/commands/pipeline.rs       # Wait logic + step progress display (primary file)
crates/cli/src/commands/pipeline_tests.rs  # Unit tests for step diffing (if test file exists)
```

No changes to:
- `crates/daemon/` (protocol, listener, query handlers)
- `crates/core/` (events, pipeline types)

## Dependencies

No new external dependencies. Uses existing:
- `oj_daemon::StepRecordDetail` — already returned by `GetPipeline`
- `oj_daemon::PipelineDetail` — already includes `steps: Vec<StepRecordDetail>` and `name`
- `format_duration` — existing helper in the same file (line 567)

## Implementation Phases

### Phase 1: Add step tracking state to the wait loop

Add a per-pipeline tracker that remembers the last-seen step history length and the last-known outcome of the current step. This allows detecting new step entries and status changes between polls.

**File:** `crates/cli/src/commands/pipeline.rs`

Add a tracking struct near `PipelineOutcome`:

```rust
/// Tracks step progress for a single pipeline during wait polling.
struct StepTracker {
    /// Number of steps we've already printed transitions for.
    printed_count: usize,
    /// Last-seen outcome of the currently-running step (to detect running→completed/failed).
    last_outcome: Option<String>,
}
```

In the wait match arm, add a tracker map alongside the existing `finished` and `canonical_ids` maps:

```rust
let mut step_trackers: HashMap<String, StepTracker> = HashMap::new();
```

**Verification:** `cargo check -p oj-cli` compiles with no behavioral change.

### Phase 2: Implement step diff and printing logic

After each `GetPipeline` response, compare the current `steps` against the tracker to detect and print transitions.

**File:** `crates/cli/src/commands/pipeline.rs`

Add a function that takes a pipeline detail and its tracker, prints any new transitions, and updates the tracker:

```rust
/// Print step transitions that occurred since the last poll.
///
/// Returns the pipeline name prefix for multi-pipeline display.
fn print_step_progress(
    detail: &oj_daemon::PipelineDetail,
    tracker: &mut StepTracker,
    show_pipeline_prefix: bool,
) {
    let prefix = if show_pipeline_prefix {
        format!("[{}] ", detail.name)
    } else {
        String::new()
    };

    for (i, step) in detail.steps.iter().enumerate() {
        if i < tracker.printed_count {
            continue; // Already printed this step's transitions
        }

        let elapsed = format_duration(step.started_at_ms, step.finished_at_ms);

        match step.outcome.as_str() {
            "running" => {
                // Only print "started" if we haven't seen this step before
                if tracker.last_outcome.is_none() || i > tracker.printed_count {
                    println!("{}{} started", prefix, step.name);
                    tracker.last_outcome = Some("running".into());
                }
            }
            "completed" => {
                // If we never saw "running", print started too
                if i >= tracker.printed_count && tracker.last_outcome.as_deref() != Some("running") {
                    println!("{}{} completed ({})", prefix, step.name, elapsed);
                } else {
                    println!("{}{} completed ({})", prefix, step.name, elapsed);
                }
                tracker.printed_count = i + 1;
                tracker.last_outcome = None;
            }
            "failed" => {
                let detail_msg = step.detail.as_deref().unwrap_or("");
                let suffix = if detail_msg.is_empty() {
                    String::new()
                } else {
                    format!(" - {}", detail_msg)
                };
                println!("{}{} failed ({}){}", prefix, step.name, elapsed, suffix);
                tracker.printed_count = i + 1;
                tracker.last_outcome = None;
            }
            "waiting" => {
                let reason = step.detail.as_deref().unwrap_or("waiting");
                println!("{}{} waiting ({})", prefix, step.name, reason);
                tracker.last_outcome = Some("waiting".into());
            }
            _ => {}
        }
    }
}
```

Call this function inside the wait loop, right after the `Some(p)` branch and before the terminal-state check:

```rust
Some(p) => {
    canonical_ids.entry(input_id.clone()).or_insert_with(|| p.id.clone());

    // Print step progress
    let tracker = step_trackers
        .entry(input_id.clone())
        .or_insert(StepTracker { printed_count: 0, last_outcome: None });
    let show_prefix = ids.len() > 1;
    print_step_progress(&p, tracker, show_prefix);

    // Existing terminal state check...
    let outcome = match p.step.as_str() { ... };
}
```

**Output format — single pipeline:**
```
init completed (0s)
plan started
plan completed (2m 44s)
implement started
implement failed (7m 32s) - shell exit code: 2
resolve started
resolve completed (8s)
push completed (0s)
```

**Output format — multiple pipelines:**
```
[auto-start-worker] init completed (0s)
[auto-start-worker] plan started
[deploy-api] setup completed (3s)
[auto-start-worker] plan completed (2m 44s)
[deploy-api] deploy started
```

**Verification:** `cargo check -p oj-cli` compiles. Manual test with a running pipeline shows step transitions printing in real-time.

### Phase 3: Handle edge cases

Handle the following edge cases in `print_step_progress`:

1. **Steps that complete between polls** — A step may go from non-existent to `completed` in a single poll (e.g., fast shell steps like `init`). Print `<step> completed (<duration>)` directly, without a separate "started" line.

2. **Initial state on first poll** — On the first poll, there may already be completed steps in the history. Print all completed/failed steps immediately so the user sees the full history caught up.

3. **`--quiet` flag (optional)** — Consider adding a `--quiet` / `-q` flag to suppress step progress and restore the original silent-wait behavior. This is a nice-to-have, not required.

Refine the diff logic: track steps by index. On each poll:
- For indices `< printed_count`: skip (already printed final state).
- For index `== printed_count` with a terminal outcome: this step finished since last poll — print the outcome line and increment `printed_count`.
- For index `== printed_count` with `running` outcome and no prior `last_outcome`: print "started" and set `last_outcome`.
- For indices `> printed_count`: multiple steps advanced in one poll — print each one's final state.

**File:** `crates/cli/src/commands/pipeline.rs`

**Verification:** Manual test with a fast pipeline (steps complete within 1s poll interval) shows correct output without duplicate lines.

### Phase 4: Add unit tests for step diffing

Add tests for the step progress diffing logic.

**File:** `crates/cli/src/commands/pipeline.rs` (or `pipeline_tests.rs` if using the `#[path]` pattern)

Test cases:

1. **No steps yet** — tracker starts at 0, empty steps list, nothing printed.
2. **Single step running** — prints "started" line.
3. **Single step completed** — prints "completed (Xs)" line.
4. **Step skipped running** — step appears directly as completed, prints only "completed" line.
5. **Multiple steps in one poll** — two steps both completed in one poll cycle, prints both.
6. **Failed step with detail** — prints failure message with detail suffix.
7. **Multi-pipeline prefix** — when `show_pipeline_prefix` is true, lines are prefixed with `[name]`.
8. **Idempotent re-polling** — calling with same state twice doesn't double-print.

To make `print_step_progress` testable, refactor it to write to a `&mut impl Write` instead of stdout directly. This allows capturing output in tests:

```rust
fn print_step_progress(
    detail: &oj_daemon::PipelineDetail,
    tracker: &mut StepTracker,
    show_pipeline_prefix: bool,
    out: &mut impl std::io::Write,
) { ... }
```

In production, pass `&mut std::io::stdout()`. In tests, pass a `Vec<u8>`.

**Verification:** `cargo test -p oj-cli` passes.

### Phase 5: Full verification

Run `make check` to ensure:
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `quench check`
- `cargo test --all`
- `cargo build --all`
- `cargo audit`
- `cargo deny check licenses bans sources`

## Key Implementation Details

### Why client-side diffing (no protocol/daemon changes)

The `PipelineDetail` response already includes `steps: Vec<StepRecordDetail>` with full history — names, timestamps, outcomes, and error details. The diff can be computed entirely on the CLI side by tracking how many steps have been printed and what the last-seen outcome was.

This avoids:
- New `Request`/`Response` variants
- Streaming connection handling in the daemon listener
- EventBus subscription plumbing
- Protocol version concerns

The 1-second polling interval means step transitions appear with at most 1 second of latency, which is acceptable for human-readable output.

### Step transition model

Steps in `step_history` are append-only and each step goes through:
```
(not present) → running → completed | failed | waiting
```

A step may skip `running` if it completes within a single poll interval. The tracker handles this by printing the terminal outcome directly.

The `StepRecordDetail` fields used:
- `name` — step name for display
- `outcome` — "running", "completed", "failed", "waiting"
- `detail` — error message (for failed) or reason (for waiting)
- `started_at_ms` / `finished_at_ms` — for duration formatting

### Duration formatting

Reuse the existing `format_duration(started_ms, finished_ms)` helper at line 567 of `pipeline.rs`. It handles the `0s`, `Xm Ys`, `Xh Ym` formatting already.

### Multi-pipeline interleaving

When `ids.len() > 1`, prefix each line with `[pipeline_name]`. Since polls happen sequentially per pipeline in the loop, output from different pipelines will naturally interleave at poll boundaries. This matches the desired output format from the instructions.

### Backward compatibility

- No new CLI flags are required (progress is always shown).
- If a `--quiet` flag is added later, it can suppress the step output without changing the API.
- Exit codes remain unchanged: 0 (success), 1 (failed), 2 (timeout), 3 (not found), 4 (cancelled).
- The existing terminal-state messages ("Pipeline X completed/failed/cancelled") continue to print as before.

## Verification Plan

1. **Compile check:** `cargo check -p oj-cli`
2. **Lint:** `cargo clippy -p oj-cli -- -D warnings`
3. **Format:** `cargo fmt --all -- --check`
4. **Unit tests:** `cargo test -p oj-cli` — step diffing tests pass
5. **Full suite:** `make check`
6. **Manual smoke tests:**
   - `oj pipeline wait <running-id>` — see step transitions as they happen
   - `oj pipeline wait <fast-id>` — completed steps shown on first poll
   - `oj pipeline wait <id1> <id2>` — `[name]` prefix on each line
   - `oj pipeline wait --all <id1> <id2>` — all pipelines' steps shown
   - `oj pipeline wait <done-id>` — shows historical steps then exits
   - Verify no duplicate lines across polls
   - Verify duration formatting is accurate
