# Plan: TTY-aware screen clearing in `oj status --watch`

## Overview

Make `oj status --watch` conditionally clear the screen based on whether stdout is a TTY. When connected to a terminal, clear the screen before each refresh (current behavior). When piped or redirected (non-TTY), just echo each frame without ANSI clear codes so output is usable by downstream tools. Refactor the watch rendering for testability and add comprehensive tests.

## Project Structure

Files to modify:

```
crates/cli/src/commands/status.rs       # TTY-conditional clearing, extract render_frame
crates/cli/src/commands/status_tests.rs # New tests for TTY-conditional behavior
```

No new files or dependencies.

## Dependencies

None beyond what's already used. `std::io::IsTerminal` is already imported in `crates/cli/src/commands/run.rs` and `crates/cli/src/color.rs`.

## Implementation Phases

### Phase 1: Refactor watch loop for testability

**Goal:** Extract the per-frame rendering logic into a testable function that accepts a `is_tty` parameter.

1. Add `use std::io::IsTerminal;` to `status.rs`.

2. Create a `render_frame` helper that prepends the clear sequence only when `is_tty` is true:

```rust
/// ANSI sequence: clear entire screen + move cursor to top-left.
const CLEAR_SCREEN: &str = "\x1B[2J\x1B[H";

/// Build one watch-mode frame.
///
/// When `is_tty` is true the frame starts with an ANSI clear-screen
/// sequence so the terminal redraws in place.  When false the content
/// is returned as-is (suitable for piped / redirected output).
fn render_frame(content: &str, is_tty: bool) -> String {
    if is_tty {
        format!("{CLEAR_SCREEN}{content}")
    } else {
        content.to_string()
    }
}
```

3. Update `handle()` to check TTY once before the loop and use `render_frame`:

```rust
pub async fn handle(args: StatusArgs, format: OutputFormat) -> Result<()> {
    if !args.watch {
        return handle_once(format, None).await;
    }

    let interval = crate::commands::pipeline::parse_duration(&args.interval)?;
    if interval.is_zero() {
        anyhow::bail!("duration must be > 0");
    }

    let is_tty = std::io::stdout().is_terminal();
    loop {
        handle_watch_frame(format, &args.interval, is_tty).await?;
        tokio::time::sleep(interval).await;
    }
}
```

4. Extract `handle_watch_frame` so the per-frame logic is separated from the infinite loop:

```rust
async fn handle_watch_frame(
    format: OutputFormat,
    interval: &str,
    is_tty: bool,
) -> Result<()> {
    let client = match DaemonClient::connect() {
        Ok(c) => c,
        Err(_) => {
            let content = format_not_running(format);
            print!("{}", render_frame(&content, is_tty));
            return Ok(());
        }
    };

    let (uptime_secs, namespaces) = match client.status_overview().await {
        Ok(data) => data,
        Err(crate::client::ClientError::DaemonNotRunning)
        | Err(crate::client::ClientError::Io(_)) => {
            // (keep existing pattern of matching specific io error kinds)
            let content = format_not_running(format);
            print!("{}", render_frame(&content, is_tty));
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    let content = match format {
        OutputFormat::Text => format_text(uptime_secs, &namespaces, Some(interval)),
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "uptime_secs": uptime_secs,
                "namespaces": namespaces,
            });
            format!("{}\n", serde_json::to_string_pretty(&obj)?)
        }
    };
    print!("{}", render_frame(&content, is_tty));

    Ok(())
}
```

5. Add a small `format_not_running` helper that returns the "not running" string (instead of printing it directly), so it can also go through `render_frame`:

```rust
fn format_not_running(format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => format!("{} not running\n", color::header("oj daemon:")),
        OutputFormat::Json => r#"{ "status": "not_running" }"#.to_string() + "\n",
    }
}
```

`handle_not_running` stays unchanged for the non-watch path (`handle_once`).

**Verify:** `cargo check --all` passes. `cargo clippy --all` clean.

### Phase 2: Add comprehensive tests

**Goal:** Thoroughly test TTY-conditional rendering, frame construction, and edge cases.

Add the following tests to `status_tests.rs`:

#### 2a. `render_frame` unit tests

```rust
#[test]
fn render_frame_tty_prepends_clear_sequence() {
    let content = "oj daemon: running 2m\n";
    let frame = render_frame(content, true);
    assert!(
        frame.starts_with(CLEAR_SCREEN),
        "TTY frame must start with clear-screen sequence"
    );
    assert!(
        frame.ends_with(content),
        "TTY frame must end with the content"
    );
}

#[test]
fn render_frame_non_tty_no_escape_codes() {
    let content = "oj daemon: running 2m\n";
    let frame = render_frame(content, false);
    assert_eq!(frame, content, "non-TTY frame should be the raw content");
    assert!(
        !frame.contains('\x1B'),
        "non-TTY frame must not contain any ANSI escape codes"
    );
}
```

#### 2b. Content identity test

Verify the status content is identical regardless of TTY mode — only the clear prefix differs.

```rust
#[test]
#[serial]
fn render_frame_content_identical_across_tty_modes() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "proj".to_string(),
        active_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "aaaa1111".to_string(),
            name: "build".to_string(),
            kind: "pipeline".to_string(),
            step: "compile".to_string(),
            step_status: "running".to_string(),
            elapsed_ms: 5000,
            waiting_reason: None,
        }],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };
    let text = format_text(60, &[ns], Some("5s"));

    let tty_frame = render_frame(&text, true);
    let non_tty_frame = render_frame(&text, false);

    // Strip the clear prefix from TTY frame; remainder must match non-TTY
    assert_eq!(&tty_frame[CLEAR_SCREEN.len()..], non_tty_frame);
}
```

#### 2c. Consecutive frames test

Ensure multiple frames concatenated in non-TTY mode are distinguishable and don't contain clear codes, while TTY frames each begin with the clear sequence.

```rust
#[test]
#[serial]
fn consecutive_frames_tty_each_start_with_clear() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let frame1_content = format_text(60, &[], Some("5s"));
    let frame2_content = format_text(120, &[], Some("5s"));

    let frame1 = render_frame(&frame1_content, true);
    let frame2 = render_frame(&frame2_content, true);

    let combined = format!("{frame1}{frame2}");

    // Count occurrences of the clear sequence
    let clear_count = combined.matches(CLEAR_SCREEN).count();
    assert_eq!(clear_count, 2, "each TTY frame must have its own clear");
}

#[test]
#[serial]
fn consecutive_frames_non_tty_no_clear_codes() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let frame1_content = format_text(60, &[], Some("5s"));
    let frame2_content = format_text(120, &[], Some("5s"));

    let frame1 = render_frame(&frame1_content, false);
    let frame2 = render_frame(&frame2_content, false);

    let combined = format!("{frame1}{frame2}");

    assert!(
        !combined.contains(CLEAR_SCREEN),
        "non-TTY output must never contain clear sequence"
    );
    // Both frames appear in order
    assert!(combined.contains("1m"));  // 60s
    assert!(combined.contains("2m"));  // 120s
}
```

#### 2d. Clear sequence constant test

Verify the constant matches the expected ANSI codes (defense against accidental edits).

```rust
#[test]
fn clear_screen_constant_is_correct_ansi() {
    assert_eq!(CLEAR_SCREEN, "\x1B[2J\x1B[H");
    assert_eq!(CLEAR_SCREEN.len(), 7);
}
```

#### 2e. `format_text` never contains clear codes

Ensure the formatting layer itself never injects clear sequences — that responsibility belongs solely to `render_frame`.

```rust
#[test]
#[serial]
fn format_text_never_contains_clear_sequence() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    // With watch interval
    let with_watch = format_text(300, &[], Some("3s"));
    assert!(
        !with_watch.contains(CLEAR_SCREEN),
        "format_text must not inject clear codes"
    );

    // Without watch interval
    let without_watch = format_text(300, &[], None);
    assert!(
        !without_watch.contains(CLEAR_SCREEN),
        "format_text must not inject clear codes"
    );
}
```

#### 2f. Non-TTY frame with rich content

End-to-end test: build a full status with pipelines, workers, queues, and agents, render as a non-TTY frame, and verify the output is clean (no escape codes beyond color — which is disabled via `NO_COLOR`).

```rust
#[test]
#[serial]
fn non_tty_frame_with_full_status_has_no_ansi_escapes() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "abcd1234".to_string(),
            name: "build".to_string(),
            kind: "pipeline".to_string(),
            step: "compile".to_string(),
            step_status: "running".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: None,
        }],
        escalated_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "efgh5678".to_string(),
            name: "deploy".to_string(),
            kind: "deploy".to_string(),
            step: "approve".to_string(),
            step_status: "waiting".to_string(),
            elapsed_ms: 120_000,
            waiting_reason: Some("needs manual approval".to_string()),
        }],
        orphaned_pipelines: vec![],
        workers: vec![oj_daemon::WorkerSummary {
            name: "builder".to_string(),
            status: "running".to_string(),
            active: 1,
            concurrency: 4,
        }],
        queues: vec![oj_daemon::QueueStatus {
            name: "tasks".to_string(),
            pending: 3,
            active: 1,
            dead: 0,
        }],
        active_agents: vec![oj_daemon::AgentStatusEntry {
            pipeline_name: "build".to_string(),
            step_name: "code".to_string(),
            agent_id: "agent-001".to_string(),
            status: "running".to_string(),
        }],
    };

    let text = format_text(600, &[ns], Some("5s"));
    let frame = render_frame(&text, false);

    assert!(!frame.contains('\x1B'), "no ANSI escapes in non-TTY + NO_COLOR frame");
    assert!(frame.contains("myproject"));
    assert!(frame.contains("pipeline"));
    assert!(frame.contains("builder"));
    assert!(frame.contains("tasks"));
    assert!(frame.contains("agent-001"));
}
```

#### 2g. TTY frame preserves ANSI color codes from content

When colors are enabled, `render_frame` in TTY mode should contain both the clear sequence AND any color codes produced by `format_text`. This verifies clear codes don't clobber content codes.

```rust
#[test]
#[serial]
fn tty_frame_preserves_color_codes_in_content() {
    std::env::remove_var("NO_COLOR");
    std::env::set_var("COLOR", "1");

    let text = format_text(120, &[], Some("5s"));
    let frame = render_frame(&text, true);

    // Starts with clear sequence
    assert!(frame.starts_with(CLEAR_SCREEN));

    // Contains color codes from format_text (header coloring)
    let after_clear = &frame[CLEAR_SCREEN.len()..];
    assert!(
        after_clear.contains("\x1b[38;5;"),
        "TTY frame should preserve color codes from content"
    );
}
```

**Verify:** `cargo test --all` passes. All new tests green.

### Phase 3: Validate and land

**Goal:** Full CI verification.

1. `cargo fmt --all`
2. `cargo clippy --all -- -D warnings`
3. `cargo build --all`
4. `cargo test --all`
5. `make check`

Manual smoke tests:
- `oj status` — unchanged one-shot behavior
- `oj status --watch` — clears screen on each refresh (TTY)
- `oj status --watch | cat` — echoes frames without clear codes (non-TTY)
- `oj status --watch 2>&1 | head -20` — output is readable without garbage escape characters
- `oj status --watch --format json | head -30` — JSON frames without clear codes when piped

## Key Implementation Details

- **TTY check location:** `std::io::stdout().is_terminal()` is evaluated once before entering the watch loop and stored in a local `is_tty: bool`. This avoids per-frame syscalls and ensures consistent behavior if stdout changes mid-run (e.g., during process lifecycle).
- **Existing pattern:** The codebase already uses `std::io::stdout().is_terminal()` in `crates/cli/src/commands/run.rs:322` and `crates/cli/src/color.rs:28`, so this follows an established convention.
- **Separation of concerns:** `format_text` produces content. `render_frame` wraps content with TTY-aware chrome (clear codes). Neither knows about the other's internals. Tests verify this boundary.
- **No separator in non-TTY mode:** Consecutive non-TTY frames are separated only by their natural trailing newlines. Adding a `---` separator was considered but rejected — downstream consumers (grep, jq, etc.) are better served by clean output. Users who want visual separation can pipe through `sed`.
- **`CLEAR_SCREEN` constant:** Extracted as a module-level `const` so tests can reference it directly. This prevents drift between the production code and test assertions.

## Verification Plan

1. **Unit tests (Phase 2):** 8 new tests covering `render_frame` in both TTY modes, content identity, consecutive frames, constant correctness, separation of concerns, rich content edge case, and color preservation.
2. **Existing tests:** All existing `status_tests.rs` and `format_text` tests continue to pass unchanged — the refactor doesn't touch `format_text`.
3. **`cargo clippy`:** No new warnings.
4. **Manual TTY vs pipe:** `oj status --watch` in a terminal clears; `oj status --watch | cat` echoes.
5. **`make check`:** Full CI suite passes.
