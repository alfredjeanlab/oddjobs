# Pipeline Wait Multi

Extend `oj pipeline wait` to accept multiple pipeline IDs with two modes: wait for **any** (default) or wait for **all** (`--all`).

## Overview

Currently `oj pipeline wait <id>` blocks until a single pipeline reaches a terminal state, using client-side polling. This plan extends it to accept multiple IDs and adds an `--all` flag, keeping the polling-based approach on the CLI side (no daemon protocol changes needed).

## Project Structure

All changes are in the CLI crate — no daemon or protocol modifications required.

```
crates/cli/src/commands/pipeline.rs   # CLI parsing + wait logic (primary file)
```

The current implementation polls via `client.get_pipeline(&id)` in a loop. The multi-wait extension polls all IDs each iteration and tracks which have reached terminal state.

## Dependencies

No new external dependencies. Uses existing:
- `clap` (already used for CLI parsing)
- `tokio::time::sleep` (already used for polling)
- `DaemonClient::get_pipeline` (existing RPC method)

## Implementation Phases

### Phase 1: Update CLI argument parsing

Modify the `PipelineCommand::Wait` variant to accept multiple IDs and an `--all` flag.

**File:** `crates/cli/src/commands/pipeline.rs`

Change:
```rust
/// Block until a pipeline reaches a terminal state
Wait {
    /// Pipeline ID or name (prefix match)
    id: String,

    /// Timeout duration (e.g. "5m", "30s", "1h")
    #[arg(long)]
    timeout: Option<String>,
},
```

To:
```rust
/// Block until pipeline(s) reach a terminal state
Wait {
    /// Pipeline IDs or names (prefix match)
    #[arg(required = true)]
    ids: Vec<String>,

    /// Wait for ALL pipelines to complete (default: wait for ANY)
    #[arg(long)]
    all: bool,

    /// Timeout duration (e.g. "5m", "30s", "1h")
    #[arg(long)]
    timeout: Option<String>,
},
```

**Verification:** `cargo check -p oj-cli` compiles (will have errors in the match arm until Phase 2).

### Phase 2: Implement multi-wait polling logic

Replace the single-pipeline polling loop with logic that handles both "any" and "all" modes.

**File:** `crates/cli/src/commands/pipeline.rs`

Replace the `PipelineCommand::Wait` match arm (lines 416–464) with:

```rust
PipelineCommand::Wait { ids, all, timeout } => {
    let timeout_dur = timeout.map(|s| parse_duration(&s)).transpose()?;
    let poll_ms = std::env::var("OJ_WAIT_POLL_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1000);
    let poll_interval = Duration::from_millis(poll_ms);
    let start = Instant::now();

    // Track which pipelines have finished and their outcomes
    let mut finished: HashMap<String, PipelineOutcome> = HashMap::new();
    // Resolve each ID to its canonical form on first successful lookup
    let mut canonical_ids: HashMap<String, String> = HashMap::new();

    loop {
        for input_id in &ids {
            if finished.contains_key(input_id) {
                continue;
            }
            let detail = client.get_pipeline(input_id).await?;
            match detail {
                None => {
                    return Err(ExitError::new(
                        3,
                        format!("Pipeline not found: {}", input_id),
                    ).into());
                }
                Some(p) => {
                    canonical_ids.entry(input_id.clone()).or_insert_with(|| p.id.clone());
                    let outcome = match p.step.as_str() {
                        "done" => Some(PipelineOutcome::Done),
                        "failed" => Some(PipelineOutcome::Failed(
                            p.error.clone().unwrap_or_else(|| "unknown error".into()),
                        )),
                        "cancelled" => Some(PipelineOutcome::Cancelled),
                        _ => None,
                    };
                    if let Some(outcome) = outcome {
                        let short_id = &canonical_ids[input_id][..8];
                        match &outcome {
                            PipelineOutcome::Done => {
                                println!("Pipeline {} ({}) completed", p.name, short_id);
                            }
                            PipelineOutcome::Failed(msg) => {
                                eprintln!("Pipeline {} ({}) failed: {}", p.name, short_id, msg);
                            }
                            PipelineOutcome::Cancelled => {
                                eprintln!("Pipeline {} ({}) was cancelled", p.name, short_id);
                            }
                        }
                        finished.insert(input_id.clone(), outcome);
                    }
                }
            }
        }

        // Check completion condition
        if all {
            if finished.len() == ids.len() {
                break;
            }
        } else if !finished.is_empty() {
            break;
        }

        // Check timeout
        if let Some(t) = timeout_dur {
            if start.elapsed() >= t {
                return Err(ExitError::new(
                    2,
                    "Timeout waiting for pipeline(s)".to_string(),
                ).into());
            }
        }

        tokio::time::sleep(poll_interval).await;
    }

    // Determine exit code from finished pipelines
    let any_failed = finished.values().any(|o| matches!(o, PipelineOutcome::Failed(_)));
    let any_cancelled = finished.values().any(|o| matches!(o, PipelineOutcome::Cancelled));
    if any_failed {
        return Err(ExitError::new(1, String::new()).into());
    }
    if any_cancelled {
        return Err(ExitError::new(4, String::new()).into());
    }
}
```

Add a helper enum (private, near the top of the file or just before `handle`):

```rust
enum PipelineOutcome {
    Done,
    Failed(String),
    Cancelled,
}
```

**Verification:** `cargo check -p oj-cli` compiles. Manual test with a single ID to verify backward compatibility.

### Phase 3: Update help text and follow-up command hints

Update the help hint in `print_pipeline_commands` (line 473) to reflect the new multi-ID syntax:

```rust
println!("    oj pipeline wait {short_id}      # Wait until pipeline ends");
```

No change needed here — the single-ID case still works. But if there are other references to `pipeline wait` in docs or help text, update them to mention multi-ID support.

**Verification:** `oj pipeline wait --help` shows correct usage with `<IDS>...` and `--all`.

### Phase 4: Add unit/integration tests

Add tests for the new behavior. Since the wait logic is client-side polling, tests should cover:

1. **Single ID (backward compat):** Verify single-ID wait still works identically.
2. **Any mode (default):** With 2+ IDs, exits as soon as one finishes. Verify the printed output includes the finished pipeline's ID/name.
3. **All mode:** With 2+ IDs, only exits when all have finished. Verify all pipelines are printed.
4. **Mixed outcomes in all mode:** One succeeds, one fails — exit code should be non-zero (1 for failure).
5. **Not found:** If any ID is not found, exit immediately with code 3.
6. **Timeout:** Verify timeout applies across the multi-wait.

These can be integration tests using the existing daemon test harness if available, or CLI-level tests that mock `DaemonClient`.

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

### Why CLI-side polling (no protocol changes)

The current `pipeline wait` uses client-side polling via `Query::GetPipeline`. Extending this to poll multiple IDs is straightforward and avoids:
- Adding new `Request`/`Response` variants to the daemon protocol
- Implementing a subscription/notification mechanism in the daemon
- Breaking protocol compatibility

The polling approach has acceptable overhead — each poll is a lightweight query, and the 1-second interval means N pipelines add N queries/second (not a scalability concern for typical usage of 2–10 IDs).

### Exit code semantics

| Condition | Exit Code |
|-----------|-----------|
| All completed pipelines succeeded | 0 |
| Any completed pipeline failed | 1 |
| Timeout before completion condition met | 2 |
| Any pipeline ID not found | 3 |
| Any completed pipeline cancelled (none failed) | 4 |

Priority: not-found (3) > failed (1) > cancelled (4) > success (0). Timeout (2) is separate.

In "any" mode, the exit code reflects only the pipeline(s) that triggered completion. In "all" mode, it reflects the worst outcome across all pipelines.

### Backward compatibility

A single `oj pipeline wait <id>` invocation behaves identically to before — `ids` will be a `Vec` with one element, `all` defaults to `false`, and the "any" logic returns immediately when that one pipeline finishes.

### Output format

Each pipeline prints its status as it finishes (even in "all" mode), so the user sees incremental progress:

```
Pipeline deploy-api (a1b2c3d4) completed
Pipeline deploy-web (e5f6g7h8) failed: build error
```

### Deduplication

If the user passes the same ID twice, the HashMap-based tracking naturally deduplicates — the second lookup will find it already in `finished` and skip it.

## Verification Plan

1. **Compile check:** `cargo check -p oj-cli`
2. **Lint:** `cargo clippy -p oj-cli -- -D warnings`
3. **Format:** `cargo fmt --all -- --check`
4. **Unit tests:** `cargo test -p oj-cli`
5. **Full suite:** `make check`
6. **Manual smoke test:**
   - `oj pipeline wait <single-id>` — backward compat
   - `oj pipeline wait <id1> <id2>` — any mode
   - `oj pipeline wait --all <id1> <id2>` — all mode
   - `oj pipeline wait <nonexistent>` — exit code 3
   - `oj pipeline wait --timeout 1s <running-id>` — exit code 2
