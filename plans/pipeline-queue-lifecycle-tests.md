# Plan: Pipeline↔Queue Lifecycle Integration Tests

## Overview

Add integration tests for the interactions between pipelines and queues — a critical gap where two recent bugs (queue items stuck active after cancel, merge pipeline cycling 7+ times) were caused by untested subsystem interactions. This creates a new test file `tests/specs/daemon/pipeline_queue.rs` with five scenarios covering cancel, failure, completion, isolation, and circuit-breaker behavior.

## Project Structure

```
tests/
├── specs.rs                          # Add module registration (1 line)
└── specs/
    └── daemon/
        └── pipeline_queue.rs         # NEW — all 5 test scenarios
```

### Key existing files (reference only, not modified):
- `tests/specs/prelude.rs` — `Project`, `CliBuilder`, `wait_for`, `SPEC_*` constants
- `tests/specs/daemon/lifecycle.rs` — daemon lifecycle test patterns
- `tests/specs/daemon/crons.rs` — async polling patterns
- `tests/e2e/merge_queue.sh` — queue/worker runbook structure (HCL reference)
- `crates/engine/src/runtime/handlers/worker.rs:574-773` — `check_worker_pipeline_complete` logic
- `crates/storage/src/state.rs:84-107` — `QueueItemStatus` enum, state transitions
- `crates/engine/src/runtime/pipeline.rs:481-619` — `cancel_pipeline` flow

## Dependencies

No new external dependencies. Tests use the existing black-box harness:
- `crate::prelude::*` — `Project`, `cli()`, `wait_for`, `SPEC_WAIT_MAX_MS`
- `oj` CLI binary (resolved by `oj_binary()`)
- `ojd` daemon binary (resolved by `ojd_binary()`)
- Persisted queues (built-in, no external services)

## Implementation Phases

### Phase 1: Scaffolding and Module Registration

**Goal:** Create the test file skeleton and register it in the test harness.

**1a. Register the module in `tests/specs.rs`:**

Add after the existing daemon module declarations (around line 33):

```rust
#[path = "specs/daemon/pipeline_queue.rs"]
mod daemon_pipeline_queue;
```

**1b. Create `tests/specs/daemon/pipeline_queue.rs` with shared constants:**

```rust
//! Pipeline↔Queue lifecycle specs
//!
//! Verify that queue items transition correctly when their associated
//! pipeline is cancelled, fails, or completes.

use crate::prelude::*;
```

Define a shared TOML runbook constant for a persisted queue with a simple shell pipeline. This runbook will be reused (with minor variations) across scenarios:

```rust
/// Runbook: persisted queue + worker + shell-only pipeline.
/// Pipeline steps: init → check → done.
/// `init` runs a command provided via the queue item's `cmd` var.
/// `check` always succeeds (echo ok).
const QUEUE_PIPELINE_RUNBOOK: &str = r#"
[queue.jobs]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = { queue = "jobs" }
handler = { pipeline = "process" }
concurrency = 1

[pipeline.process]
vars = ["cmd"]

[[pipeline.process.step]]
name = "work"
run = "${var.item.cmd}"

[[pipeline.process.step]]
name = "done"
run = "echo done"
"#;
```

Define a variant with higher concurrency for multi-item tests:

```rust
/// Same as QUEUE_PIPELINE_RUNBOOK but concurrency = 3.
const QUEUE_PIPELINE_CONCURRENT_RUNBOOK: &str = r#"
[queue.jobs]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = { queue = "jobs" }
handler = { pipeline = "process" }
concurrency = 3

[pipeline.process]
vars = ["cmd"]

[[pipeline.process.step]]
name = "work"
run = "${var.item.cmd}"

[[pipeline.process.step]]
name = "done"
run = "echo done"
"#;
```

**Milestone:** `cargo test --test specs daemon_pipeline_queue` compiles with no test functions yet.

---

### Phase 2: Test 1 — Cancel Transitions Queue Item

**Goal:** Verify that `oj pipeline cancel` transitions the queue item out of `active` status.

**Scenario:** Push an item whose pipeline step blocks (e.g., `sleep 30`). Cancel the pipeline. Assert the queue item is no longer `active`.

```rust
#[test]
fn cancel_pipeline_transitions_queue_item_from_active() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a blocking command so the pipeline stays on "work" step
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "sleep 30"}"#])
        .passes();

    // Wait for the pipeline to reach "running" on the "work" step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("work") && out.contains("running")
    });
    assert!(running, "pipeline should be running the work step");

    // Verify queue item is active
    let active = temp.oj().args(&["queue", "items", "jobs"]).passes().stdout();
    assert!(active.contains("active"), "queue item should be active");

    // Get pipeline ID from pipeline list output, then cancel it
    let pipeline_list = temp.oj().args(&["pipeline", "list"]).passes().stdout();
    let pipeline_id = extract_pipeline_id(&pipeline_list);
    temp.oj()
        .args(&["pipeline", "cancel", &pipeline_id])
        .passes()
        .stdout_has("Cancelled");

    // Wait for queue item to leave "active" status
    let transitioned = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["queue", "items", "jobs"]).passes().stdout();
        !out.contains("active")
    });

    if !transitioned {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        transitioned,
        "queue item must not stay active after pipeline cancel"
    );

    // Item should be failed or dead (not active, not pending without retry config)
    let final_status = temp.oj().args(&["queue", "items", "jobs"]).passes().stdout();
    assert!(
        final_status.contains("dead") || final_status.contains("failed"),
        "cancelled pipeline should mark queue item as dead or failed, got: {}",
        final_status
    );
}
```

A helper function will be needed to extract pipeline IDs from `oj pipeline list` output:

```rust
/// Extract the first pipeline ID from `oj pipeline list` output.
fn extract_pipeline_id(pipeline_list_output: &str) -> String {
    // Pipeline list shows IDs in first column — parse accordingly.
    // The ID is the pipeline name (e.g., "process") — but we need the
    // unique pipeline instance ID. Use `oj pipeline list --json` if available,
    // or parse the tabular output.
    //
    // Implementation note: check the exact output format of `oj pipeline list`
    // and extract the ID field. Likely the first non-header token on each data row.
    todo!("implement based on actual pipeline list output format")
}
```

> **Implementation note:** The helper `extract_pipeline_id` needs to parse the actual `oj pipeline list` output. Check the format in `crates/cli/src/commands/pipeline.rs` (the `List` subcommand). If `--json` output is supported, prefer that for reliable parsing. Otherwise, split lines and extract the ID column.

**Milestone:** Test passes — cancel causes queue item to leave `active`.

---

### Phase 3: Tests 2 & 3 — Failure → Dead, Success → Completed

**Goal:** Verify terminal pipeline outcomes map to correct queue item statuses.

#### Test 2: Pipeline failure marks queue item dead

When a pipeline fails on its terminal step (reaches `step = "failed"`), and there's no retry config, the queue item should be marked `dead`.

```rust
#[test]
fn failed_pipeline_marks_queue_item_dead() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a command that will fail (exit 1)
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "exit 1"}"#])
        .passes();

    // Wait for queue item to reach dead status (no retry config → immediate dead)
    let dead = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["queue", "items", "jobs"]).passes().stdout();
        out.contains("dead")
    });

    if !dead {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(dead, "failed pipeline should mark queue item as dead");

    // Pipeline should show failed terminal state
    temp.oj()
        .args(&["pipeline", "list"])
        .passes()
        .stdout_has("failed");
}
```

#### Test 3: Successful pipeline marks queue item completed and frees concurrency

```rust
#[test]
fn completed_pipeline_marks_queue_item_completed() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a command that succeeds
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo hello"}"#])
        .passes();

    // Wait for queue item to reach completed status
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["queue", "items", "jobs"]).passes().stdout();
        out.contains("completed")
    });

    if !completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        completed,
        "successful pipeline should mark queue item as completed"
    );

    // Pipeline should show completed terminal state
    temp.oj()
        .args(&["pipeline", "list"])
        .passes()
        .stdout_has("completed");

    // Verify concurrency slot is freed by pushing another item
    // (worker concurrency = 1, so a second item can only run if the slot was freed)
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo second"}"#])
        .passes();

    let second_completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["queue", "items", "jobs"]).passes().stdout();
        // Both items should be completed
        out.matches("completed").count() >= 2
    });
    assert!(
        second_completed,
        "second item should complete, proving concurrency slot was freed"
    );
}
```

**Milestone:** Both tests pass — failure→dead and success→completed transitions verified.

---

### Phase 4: Test 4 — Multi-Item Isolation

**Goal:** Verify that when one pipeline fails, other active pipelines/queue items are unaffected.

This test uses the concurrent runbook (concurrency = 3):

```rust
#[test]
fn one_pipeline_failure_does_not_affect_others() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(".oj/runbooks/queue.toml", QUEUE_PIPELINE_CONCURRENT_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push 3 items: one fast-fail, two that succeed (with a brief delay to stay active)
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "exit 1"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo ok"}"#])
        .passes();
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "echo ok"}"#])
        .passes();

    // Wait for all 3 items to reach terminal status
    let all_terminal = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["queue", "items", "jobs"]).passes().stdout();
        !out.contains("active") && !out.contains("pending")
    });

    if !all_terminal {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(all_terminal, "all queue items should reach terminal status");

    // Verify: 1 dead (the failed one), 2 completed
    let items_output = temp.oj().args(&["queue", "items", "jobs"]).passes().stdout();
    assert_eq!(
        items_output.matches("completed").count(),
        2,
        "two items should be completed"
    );
    assert!(
        items_output.contains("dead") || items_output.contains("failed"),
        "the failing item should be dead or failed"
    );
}
```

**Milestone:** Test passes — failure isolation between concurrent queue items verified.

---

### Phase 5: Test 5 — Circuit Breaker (Action Attempts Escalation)

**Goal:** Verify that when a pipeline exhausts its action attempts (cycling through `on_fail` handlers), it escalates rather than cycling indefinitely.

The current cycle-prevention mechanism is `action_attempts` tracking with configurable `attempts` limits on agent actions. When attempts are exhausted, the pipeline escalates to `waiting` status for human intervention.

> **Note:** The instructions reference "step_visits tracking." If a new `step_visits` counter is implemented separately from the existing `action_attempts` mechanism, this test should be adapted to exercise that path instead. The test below exercises the existing escalation-on-exhaustion behavior, which is the same semantic the instructions describe.

This test requires an agent-based pipeline since `action_attempts` is tracked for agent lifecycle actions (`on_idle`, `on_dead`). Use a `claudeless` scenario with `-p` (print mode) and `on_dead = { action = "recover", attempts = 2 }` to limit retries:

```rust
/// Runbook where the agent always fails (exits immediately via -p mode),
/// and on_dead is configured with limited recover attempts.
/// After exhausting attempts, the pipeline should escalate.
fn circuit_breaker_runbook(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[queue.jobs]
type = "persisted"
vars = ["cmd"]

[worker.runner]
source = {{ queue = "jobs" }}
handler = {{ pipeline = "process" }}
concurrency = 1

[pipeline.process]
vars = ["cmd"]

[[pipeline.process.step]]
name = "work"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "This will fail."
on_dead = {{ action = "recover", attempts = 2 }}
"#,
        scenario_path.display()
    )
}

/// Scenario that makes the agent exit immediately with a failure indicator.
const FAILING_AGENT_SCENARIO: &str = r#"
name = "failing-agent"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "I cannot complete this task."

[tool_execution]
mode = "live"
"#;
```

```rust
#[test]
fn circuit_breaker_escalates_after_max_attempts() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/fail.toml", FAILING_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/fail.toml");
    temp.file(
        ".oj/runbooks/queue.toml",
        &circuit_breaker_runbook(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push an item — the agent will exit, recover, exit, recover, then escalate
    temp.oj()
        .args(&["queue", "push", "jobs", r#"{"cmd": "noop"}"#])
        .passes();

    // Wait for pipeline to reach "waiting" (escalated) status
    let escalated = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        let out = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        out.contains("waiting")
    });

    if !escalated {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        escalated,
        "pipeline should escalate to waiting after exhausting recover attempts"
    );

    // Queue item should still be active (pipeline hasn't terminated, it's waiting)
    let items = temp.oj().args(&["queue", "items", "jobs"]).passes().stdout();
    assert!(
        items.contains("active"),
        "queue item should remain active while pipeline is waiting for intervention"
    );
}
```

**Milestone:** Test passes — pipeline escalates instead of cycling after exhausting attempts.

---

### Phase 6: Polish and Edge Cases

**Goal:** Finalize helpers, ensure all tests pass together, verify performance budget.

1. **Implement `extract_pipeline_id` helper** — parse `oj pipeline list` output to extract the pipeline instance ID. Check if `oj pipeline list` supports `--json` for reliable parsing; if not, parse the tabular text output. Alternatively, capture the pipeline ID from the `oj queue push` or `oj run` output if it prints one.

2. **Verify cancel-with-retry variant** — optionally add a subcase to Test 1 using a runbook with `retry.attempts = 2`. After cancel, the item should transition Failed → (retry timer) → Pending, demonstrating the item isn't stuck active. This may be folded into Test 1 or split into its own test function.

3. **Run full suite** and verify performance:
   - Each test should complete within the budget (< 350ms for passing, ~2s timeout for async waits)
   - `cargo test --test specs daemon_pipeline_queue` runs all 5 tests

4. **Run `make check`** — format, clippy, build, test, deny.

**Milestone:** All tests green, `make check` passes.

## Key Implementation Details

### Runbook Pattern

All tests use **persisted queues** with **shell-only pipelines** (no real agents needed for tests 1-4). This keeps tests fast — no tmux sessions, no claudeless binaries. Test 5 requires a claudeless agent to exercise the `on_dead` action attempt tracking.

### Queue Item Status Lifecycle

The queue item status enum (`QueueItemStatus`) has these variants, which map to lowercase strings in CLI output:

| Status | Meaning |
|--------|---------|
| `pending` | Waiting to be picked up by a worker |
| `active` | Pipeline is running for this item |
| `completed` | Pipeline reached `step = "done"` |
| `failed` | Pipeline reached a non-done terminal step; may retry |
| `dead` | Failed and exhausted retries (or no retry config) |

### Pipeline Terminal Steps

A pipeline is terminal when `step` is one of: `done`, `failed`, `cancelled`.

- `done` → `QueueCompleted` event → item status = `completed`
- `failed` or `cancelled` → `QueueFailed` event → item status = `failed` → retry-or-dead logic

### Worker Pipeline Completion Flow

When a pipeline reaches a terminal step (`check_worker_pipeline_complete` in `crates/engine/src/runtime/handlers/worker.rs:574`):

1. Pipeline removed from worker's `active_pipelines` set (frees concurrency slot)
2. Pipeline removed from `item_pipeline_map`
3. For persisted queues:
   - If `step == "done"`: emit `QueueCompleted`
   - Otherwise: emit `QueueFailed`, then check retry config
   - If retries remaining: schedule retry timer
   - If retries exhausted (or no retry config): emit `QueueItemDead`
4. If worker has capacity: poll for next item

### Avoiding Flakiness

- Use `wait_for(SPEC_WAIT_MAX_MS, || ...)` for all async state checks — never `thread::sleep`
- Use blocking commands (`sleep 30`) for tests that need the pipeline to stay running (Test 1)
- Use immediately-completing commands (`echo ok`, `exit 1`) for tests that need fast terminal states (Tests 2-4)
- Print `daemon_log()` on assertion failure for debugging

### Helper: Pipeline ID Extraction

The `extract_pipeline_id` function must parse the pipeline ID from CLI output. Two approaches:

1. **Preferred**: If `oj pipeline list` supports `--format json` or `--json`, parse the JSON array and extract the `id` field.
2. **Fallback**: Parse the tabular output — the pipeline ID appears in the first column or can be extracted from a pattern like `process-XXXXXXXX`.

Check `crates/cli/src/commands/pipeline.rs` for the `List` subcommand's output format.

## Verification Plan

### Running Tests

```bash
# Run just the new tests
cargo test --test specs daemon_pipeline_queue

# Run with output for debugging
cargo test --test specs daemon_pipeline_queue -- --nocapture

# Run the full spec suite (should still be < 5s)
cargo test --test specs
```

### What Each Test Verifies

| # | Test | Asserts | Bug prevented |
|---|------|---------|---------------|
| 1 | `cancel_pipeline_transitions_queue_item_from_active` | Queue item leaves `active` after `oj pipeline cancel` | Items stuck active after cancel |
| 2 | `failed_pipeline_marks_queue_item_dead` | Failed pipeline → item `dead` (no retry config) | Items stuck active after failure |
| 3 | `completed_pipeline_marks_queue_item_completed` | Successful pipeline → item `completed`; concurrency slot freed | Slot leak preventing next item |
| 4 | `one_pipeline_failure_does_not_affect_others` | One failure among concurrent items doesn't break others | Cross-item interference |
| 5 | `circuit_breaker_escalates_after_max_attempts` | Pipeline escalates to `waiting` after exhausting `on_dead` attempts | Infinite cycling (7+ retry bug) |

### Pre-Commit Checklist

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --all -- -D warnings`
- [ ] `cargo build --all`
- [ ] `cargo test --all`
- [ ] `make check` passes
