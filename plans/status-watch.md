# Plan: `oj status --watch`

## Overview

Add `--watch` and `--interval` flags to `oj status` that re-run the status display in a loop, clearing the screen between refreshes. This is a simple watch loop (not a TUI) — Ctrl+C to exit.

## Project Structure

Files to modify:

```
crates/cli/src/main.rs           # Change Status variant from unit to struct with args
crates/cli/src/commands/status.rs # Add watch loop, accept new args
```

No new files needed.

## Dependencies

None. Uses only `std::time::Duration`, `tokio::time::sleep`, and existing `parse_duration` from `pipeline.rs`.

## Implementation Phases

### Phase 1: Add CLI flags to the Status command

**Goal:** Parse `--watch` and `--interval` flags via clap.

1. In `main.rs`, change the `Status` variant from a unit variant to a struct variant (or a wrapper around a new `StatusArgs` struct):

```rust
/// Show overview of active work across all projects
Status(status::StatusArgs),
```

2. In `status.rs`, define the args struct:

```rust
#[derive(clap::Args)]
pub struct StatusArgs {
    /// Re-run status display in a loop (Ctrl+C to exit)
    #[arg(long)]
    pub watch: bool,

    /// Refresh interval for --watch mode (e.g. 2s, 10s)
    #[arg(long, default_value = "5s")]
    pub interval: String,
}
```

3. Update the dispatch in `main.rs` to pass args:

```rust
Commands::Status(args) => {
    status::handle(args, format).await?;
}
```

**Verify:** `cargo check --all` passes.

### Phase 2: Implement watch loop

**Goal:** When `--watch` is set, loop: clear screen → print status → sleep interval.

1. Update `handle()` signature to accept `StatusArgs`.

2. Parse the interval string using the existing `parse_duration` from `pipeline.rs` (make it `pub(crate)` if not already). Validate the interval before entering the loop — fail early if the duration string is invalid.

3. If `--watch` is not set, run the existing one-shot logic and return.

4. If `--watch` is set, enter a loop:

```rust
let interval = crate::commands::pipeline::parse_duration(&args.interval)?;
loop {
    // Clear screen: print ANSI escape "\x1B[2J\x1B[H"
    print!("\x1B[2J\x1B[H");
    // Run existing status display (one-shot)
    handle_once(format).await?;
    tokio::time::sleep(interval).await;
}
```

5. Extract the existing one-shot status logic into a `handle_once(format)` helper so both paths share the same code.

6. Ctrl+C is handled automatically by the tokio runtime (terminates the process).

**Verify:** `cargo check --all` passes. Manual test: `oj status --watch` clears and refreshes, `oj status --watch --interval 2s` refreshes every 2 seconds, Ctrl+C exits cleanly.

### Phase 3: Validate and land

**Goal:** Ensure all checks pass.

1. Run `make check` (fmt, clippy, tests, build, audit, deny).
2. Fix any warnings (e.g., unused imports if the `StatusArgs` struct needs adjustments).
3. Commit with message: `feat(cli): add --watch and --interval flags to oj status`

## Key Implementation Details

- **Screen clearing:** Use ANSI escape `\x1B[2J\x1B[H` (clear screen + move cursor to top-left). This works on all modern terminals. No dependency needed.
- **Duration parsing:** Reuse the existing `parse_duration()` from `crates/cli/src/commands/pipeline.rs` (line 130). It already supports `2s`, `10s`, `5m`, etc. `parse_duration` is already `pub` so it's accessible from within the `commands` module.
- **No `--interval` without `--watch`:** The `--interval` flag only takes effect when `--watch` is also set. No need to enforce this with clap `requires` — just document it and ignore the value when not watching.
- **Graceful degradation:** In watch mode, if the daemon goes down mid-loop, the "not running" message appears on the next refresh (existing `handle_not_running` logic). No special handling needed.
- **JSON format + watch:** Works fine — each refresh prints a full JSON object after clearing the screen. This is useful for scripting with `watch`-like tools, though the primary use case is text mode.

## Verification Plan

1. **`cargo check --all`** — compiles without errors
2. **`cargo clippy --all-targets --all-features -- -D warnings`** — no warnings
3. **`cargo test --all`** — existing tests pass (no new tests needed for a simple CLI flag; the watch loop is trivial)
4. **Manual testing:**
   - `oj status` — unchanged behavior (one-shot)
   - `oj status --watch` — clears and refreshes every 5s
   - `oj status --watch --interval 2s` — refreshes every 2s
   - `oj status --watch --interval 0` — fails with "duration must be > 0"
   - `oj status --watch --interval abc` — fails with parse error
   - Ctrl+C during watch — exits cleanly
   - Daemon not running + watch — shows "not running" each refresh
5. **`make check`** — full verification suite passes
