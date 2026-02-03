# Plan: Colorize Data Output Views

## Overview

Extend ANSI color support from clap help text to structured data output in list, show, and status commands. The `color` module already provides `header()`, `literal()`, `context()`, and `muted()` helpers (with `#[allow(dead_code)]` + `KEEP UNTIL: list/status command coloring` markers). This plan wires those helpers into the text-mode output paths of every command that displays tabular or detail views, and adds status-aware coloring (green/yellow/red badges).

## Project Structure

```
crates/cli/src/
├── color.rs              # MODIFIED — add status_color(), remove dead_code markers
├── color_tests.rs        # MODIFIED — add tests for status coloring
├── output.rs             # UNCHANGED
├── commands/
│   ├── status.rs         # MODIFIED — colorize dashboard output
│   ├── pipeline.rs       # MODIFIED — colorize list + show
│   ├── pipeline_wait.rs  # UNCHANGED (step progress is ephemeral, not a data view)
│   ├── agent.rs          # MODIFIED — colorize list + show
│   ├── session.rs        # MODIFIED — colorize list
│   ├── queue.rs          # MODIFIED — colorize list + items
│   ├── worker.rs         # MODIFIED — colorize list
│   ├── cron.rs           # MODIFIED — colorize list
│   ├── decision.rs       # MODIFIED — colorize list + show
│   ├── workspace.rs      # MODIFIED — colorize list + show
│   ├── project.rs        # MODIFIED — colorize list
│   ├── resolve.rs        # MODIFIED — colorize show (agent/session detail)
│   └── run.rs            # UNCHANGED (already uses HelpPrinter for colored help)
└── ...
```

## Dependencies

No new crate dependencies. The existing `color.rs` provides raw ANSI 256-color escapes via `fg256()` and the four palette helpers. Status badge colors use standard ANSI 16-color codes (green=32, yellow=33, red=31) which render well on all terminals.

## Implementation Phases

### Phase 1: Extend the color module with status badge support

Add status-aware coloring to `color.rs`. Status values map to semantic colors:

```rust
pub mod codes {
    // ... existing HEADER, LITERAL, CONTEXT, MUTED ...

    // Status badge colors (standard ANSI)
    pub const GREEN: u8 = 2;    // 16-color green
    pub const YELLOW: u8 = 3;   // 16-color yellow
    pub const RED: u8 = 1;      // 16-color red
}

/// Colorize a status string based on its semantic meaning.
///
/// - Green: completed, done, running (healthy active states)
/// - Yellow: waiting, escalated, pending, idle, orphaned
/// - Red: failed, cancelled, dead, gone
/// - Default (no color): unknown states
pub fn status(text: &str) -> String {
    if !should_colorize() {
        return text.to_string();
    }
    let lower = text.to_lowercase();
    let code = match lower.as_str() {
        "completed" | "done" | "running" | "started" => "\x1b[32m",
        "waiting" | "escalated" | "pending" | "idle"
        | "orphaned" | "stopping" | "stopped" => "\x1b[33m",
        "failed" | "cancelled" | "dead" | "gone" | "error" => "\x1b[31m",
        _ => return text.to_string(),
    };
    format!("{code}{text}{RESET}")
}
```

Remove all `#[allow(dead_code)]` annotations and `// KEEP UNTIL` comments from the existing helpers (`header()`, `literal()`, `context()`, `muted()`, `codes::MUTED`) since they will now be used.

**Files**: `crates/cli/src/color.rs`, `crates/cli/src/color_tests.rs`

### Phase 2: Colorize table headers and IDs in list commands

Apply `color::header()` to column header rows and `color::muted()` to IDs across all list commands. The pattern is consistent: header row gets `header()`, short IDs get `muted()`.

Only `OutputFormat::Text` paths are modified. JSON output is never colored.

**Pattern** (applied to each list command):

```rust
// Before:
println!("{:<w_id$} {:<w_name$} STATUS", "ID", "NAME");

// After:
use crate::color;
println!(
    "{} {} STATUS",
    color::header(&format!("{:<w_id$}", "ID")),
    color::header(&format!("{:<w_name$}", "NAME")),
);
```

For data rows, IDs are muted and status values are colorized:

```rust
// Before:
println!("{:<w_id$} {:<w_name$} {}", id, p.name, p.step_status);

// After:
println!(
    "{} {:<w_name$} {}",
    color::muted(&format!("{:<w_id$}", id)),
    p.name,
    color::status(&p.step_status),
);
```

**Commands to modify**:
- `pipeline.rs` — `format_pipeline_list()`: header row + data rows (6 branch variants for project/retries combinations)
- `agent.rs` — `AgentCommand::List` handler: header row + data rows
- `session.rs` — `format_session_list()`: header row + data rows
- `queue.rs` — `QueueCommand::List` and `QueueCommand::Items`: header rows + data rows (status in items)
- `worker.rs` — `WorkerCommand::List`: header row + data rows (status column)
- `cron.rs` — `CronCommand::List`: header row + data rows (status column)
- `decision.rs` — `DecisionCommand::List`: header row + data rows
- `workspace.rs` — `WorkspaceCommand::List`: header row + data rows (status column)
- `project.rs` — `handle_list()`: header row + data rows

**Files**: All command files listed above.

### Phase 3: Colorize show/detail views

Apply colors to the key-value detail views in show commands. The pattern:
- Entity type + ID label: `color::header()` (e.g., "Pipeline: abc12345")
- Field labels: `color::context()` (e.g., "  Name:", "  Status:")
- Status values: `color::status()`
- Section headers within detail views: `color::header()` (e.g., "  Steps:", "  Agents:")

```rust
// Before:
println!("Pipeline: {}", p.id);
println!("  Name: {}", p.name);
println!("  Status: {}", p.step_status);
println!("  Steps:");

// After:
println!("{} {}", color::header("Pipeline:"), p.id);
println!("  {} {}", color::context("Name:"), p.name);
println!("  {} {}", color::context("Status:"), color::status(&p.step_status));
println!("  {}", color::header("Steps:"));
```

**Commands to modify**:
- `pipeline.rs` — `PipelineCommand::Show`: pipeline detail, steps list, agents list, vars
- `agent.rs` — `AgentCommand::Show`: agent detail
- `decision.rs` — `DecisionCommand::Show`: decision detail, context, options
- `workspace.rs` — `WorkspaceCommand::Show`: workspace detail
- `resolve.rs` — `handle_show()`: agent and session detail branches

**Files**: `pipeline.rs`, `agent.rs`, `decision.rs`, `workspace.rs`, `resolve.rs`

### Phase 4: Colorize the status dashboard

The `oj status` dashboard (`status.rs`) has the richest formatting. Apply colors to:
- Top-level summary line: `color::header("oj daemon:")` prefix, green for "up", counts in context color
- Namespace separator lines (`── project ──`): `color::header()`
- Section labels ("Pipelines (3 active):", "Workers:", etc.): `color::header()`
- Status indicators: `color::status()` for step statuses
- IDs: `color::muted()` for short pipeline/agent IDs
- Warning symbols (⚠): `color::status("escalated")` wrapping

```rust
// Namespace header
let label_colored = color::header(&format!("── {} ", label));
let _ = write!(out, "{}", label_colored);
// ... padding ...

// Section header
let _ = writeln!(out, "  {}", color::header(&format!("Pipelines ({} active):", n)));

// Worker status indicator
let indicator_colored = if w.status == "running" {
    color::status("●")  // green
} else {
    color::muted("○")   // grey
};
```

**Files**: `crates/cli/src/commands/status.rs`

### Phase 5: Update tests and remove dead-code markers

1. **Add tests for `status()` helper** in `color_tests.rs`:
   - `status_green_for_running`, `status_yellow_for_waiting`, `status_red_for_failed`
   - `status_plain_when_no_color`
   - `status_unknown_returns_plain`

2. **Update existing list/show tests** that check exact output strings. Tests use `format_pipeline_list(&mut buf, ...)` which writes to a `Vec<u8>`. Since `should_colorize()` checks `NO_COLOR`, `COLOR`, and TTY, tests running in CI (not a TTY, no COLOR env) will produce plain output by default — existing assertions should still pass. Verify this and add a note if any tests need `NO_COLOR=1`.

3. **Remove all `KEEP UNTIL: list/status command coloring` comments** and their associated `#[allow(dead_code)]` annotations from `color.rs`.

**Files**: `crates/cli/src/color.rs`, `crates/cli/src/color_tests.rs`

## Key Implementation Details

### Color application rules

| Element | Color function | Rationale |
|---------|---------------|-----------|
| Table header row labels (ID, NAME, STATUS, ...) | `color::header()` (steel blue, 74) | Visual anchor, matches clap help headers |
| Entity IDs (short hex IDs) | `color::muted()` (dark grey, 240) | IDs are reference data, not primary info |
| Status values (Running, Failed, etc.) | `color::status()` (green/yellow/red) | Semantic meaning at a glance |
| Detail view field labels (Name:, Status:, ...) | `color::context()` (medium grey, 245) | De-emphasize labels, emphasize values |
| Detail view section headers (Steps:, Agents:, ...) | `color::header()` (steel blue, 74) | Group separator |
| Data values (names, paths, counts) | No color (default terminal foreground) | Primary content stays uncolored |

### Color bypass for JSON output

All coloring is applied only in `OutputFormat::Text` match arms. JSON paths are unchanged. The `color::status()` and other helpers are only called inside `OutputFormat::Text` branches.

### Test compatibility

`should_colorize()` returns `false` when stdout is not a TTY (the standard CI case). Tests that write to `Vec<u8>` buffers via `format_pipeline_list()` don't even go through stdout — they call `writeln!` on a buffer. Since the color helpers check `should_colorize()` internally, and test environments are not TTYs, colored output won't appear in test assertions unless `COLOR=1` is explicitly set. Existing test assertions comparing exact string output remain valid.

### Status mapping completeness

The `status()` function uses a lowercase match so "Running", "RUNNING", and "running" all map to green. Unknown status values pass through uncolored, which is safe for forward compatibility when new statuses are added.

### Consistent worker status indicator

The dashboard's `●`/`○` worker indicators already convey running/stopped semantically via filled/unfilled. Adding green/grey coloring reinforces this without changing the character.

## Verification Plan

1. **`make check`** — must pass (fmt, clippy, tests, build, deny)
2. **Manual verification** (all commands, with and without color):
   - `oj pipeline list` — header row in steel blue, IDs in dark grey, statuses colored
   - `oj pipeline show <id>` — field labels in grey, status in green/red/yellow
   - `oj agent list` — same pattern as pipeline list
   - `oj agent show <id>` — field labels colored
   - `oj status` — dashboard with colored sections, status badges, muted IDs
   - `oj session list` — header row colored
   - `oj queue list` / `oj queue items <q>` — headers + status colored
   - `oj worker list` — headers + status colored
   - `oj cron list` — headers + status colored
   - `oj decision list` / `oj decision show <id>` — colored
   - `oj workspace list` / `oj workspace show <id>` — colored
   - `oj project list` — header row colored
   - `oj show <id>` — resolved entity show is colored
   - `NO_COLOR=1 oj pipeline list` — plain uncolored output
   - `oj pipeline list -o json` — no ANSI codes in JSON output
   - `oj pipeline list | cat` — plain output (not a TTY)
   - `COLOR=1 oj pipeline list | cat` — colored even when piped (forced)
3. **Unit tests** — `color_tests.rs` covers `status()` helper variants and disabled-color paths
4. **Existing tests pass** — `cargo test --all` with no regressions
