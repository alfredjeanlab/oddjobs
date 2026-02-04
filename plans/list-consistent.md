# Consistent Table Renderer for CLI List Views

## Overview

Build a shared `Table` renderer in `crates/cli/src/table.rs` that all list commands use for tabular output. This eliminates per-command formatting logic, fixes `oj queue list` alignment issues (tab-separated values), and ensures consistent color application, column widths, and alignment across every list view.

Currently, 7 commands render tables independently:
- **`oj pipeline list`** — good pattern (dynamic widths, space-padded), but verbose/duplicated
- **`oj cron list`** — same good pattern, duplicated
- **`oj worker list`** — good pattern, but color applied inside format padding (misalignment risk)
- **`oj project list`** — good pattern with right-aligned numeric columns
- **`oj agent list`** — fixed column widths, truncation, same color/padding issue as worker
- **`oj queue list`** — **broken**: uses `\t` separators, no width calculation
- **`oj queue items`** — **broken**: uses `\t` separators, inconsistent `key=value` in data cells

## Project Structure

```
crates/cli/src/
├── table.rs          # NEW — shared table renderer
├── table_tests.rs    # NEW — unit tests for table renderer
├── color.rs          # UNCHANGED — existing color palette
├── output.rs         # UNCHANGED
├── main.rs           # MODIFIED — add `mod table;`
└── commands/
    ├── pipeline.rs   # MODIFIED — replace inline formatting with Table
    ├── cron.rs       # MODIFIED — replace inline formatting with Table
    ├── worker.rs     # MODIFIED — replace inline formatting with Table
    ├── project.rs    # MODIFIED — replace inline formatting with Table
    ├── agent.rs      # MODIFIED — replace inline formatting with Table
    ├── queue.rs      # MODIFIED — replace inline formatting with Table
    └── ...
```

## Dependencies

No new external dependencies. The renderer uses:
- `std::fmt::Write` for buffered output
- `crate::color` for the existing color palette

## Implementation Phases

### Phase 1: Build the `Table` struct and renderer

Create `crates/cli/src/table.rs` with a builder-style API.

**Core types:**

```rust
pub enum Align {
    Left,
    Right,
}

pub enum CellStyle {
    /// No color applied
    Plain,
    /// Apply color::header() — for column headers (automatic)
    Header,
    /// Apply color::muted()
    Muted,
    /// Apply color::status() — auto-detects green/yellow/red
    Status,
    /// Apply color::context()
    Context,
}

pub struct Column {
    pub name: &'static str,
    pub align: Align,
    pub style: CellStyle,
    /// Minimum width (defaults to header text length)
    pub min_width: Option<usize>,
    /// Maximum width (None = unlimited)
    pub max_width: Option<usize>,
}

pub struct Table {
    columns: Vec<Column>,
    rows: Vec<Vec<String>>,
}
```

**Key API:**

```rust
impl Table {
    pub fn new(columns: Vec<Column>) -> Self;
    pub fn row(&mut self, cells: Vec<String>);
    /// Render the full table (header + rows) to the given writer.
    /// Column widths are auto-computed from data.
    /// The last column is never padded.
    pub fn render(&self, out: &mut impl Write);
}
```

**Column shorthand constructors:**

```rust
impl Column {
    pub fn left(name: &'static str) -> Self;        // Left-aligned, Plain style
    pub fn right(name: &'static str) -> Self;       // Right-aligned, Plain style
    pub fn muted(name: &'static str) -> Self;       // Left-aligned, Muted style
    pub fn status(name: &'static str) -> Self;      // Left-aligned, Status style
}
```

**Rendering algorithm:**

1. Compute each column's display width: `max(min_width.unwrap_or(header.len()), max_data_len)`, capped by `max_width` if set.
2. Print the header row: each header cell padded to its column width, styled with `color::header()`.
3. Print data rows: each cell padded to column width using `{:<w$}` or `{:>w$}` based on alignment. Padding is applied to the **plain text** first, then color codes wrap the padded string — this ensures ANSI escapes don't corrupt width calculations.
4. The rightmost column is never padded (no trailing whitespace).
5. Columns are separated by a double-space `"  "` gap.
6. Values exceeding `max_width` are truncated with no ellipsis (matching existing `truncate()` behavior).

**Color-aware padding (critical fix):**

The current bug in `worker list` and `agent list` is that color is applied *around* `format!("{:<w$}", text)`, making `{:<w$}` count invisible ANSI bytes. The table renderer fixes this by always padding first, then wrapping:

```rust
// CORRECT: pad plain text, then colorize
let padded = format!("{:<width$}", text);
let styled = color::status(&padded);

// WRONG (current worker/agent code): colorize includes invisible bytes
// let styled = color::status(&format!("{:<width$}", text));
// This works by accident only when format! pads BEFORE colorize wraps it,
// but the resulting String has invisible bytes that confuse subsequent padding.
```

Actually, in the current code `color::status(&format!("{:<width$}", text))` does pad first then wrap — so the padding is correct for that column. The problem is that the *next* column's `{:<w$}` in the same `println!` format string sees the colored string's full byte length. The table renderer avoids this entirely by building each cell independently and concatenating with separators.

**Write tests in `table_tests.rs`:**

- Empty table prints nothing (or just headers — decide: print headers only if rows exist)
- Single row, single column
- Multi-column left/right alignment
- Column width adapts to widest cell
- `max_width` truncates long values
- `min_width` enforces minimum even for short data
- Last column has no trailing padding
- CellStyle::Muted / Status apply correct ANSI codes (test with `COLOR=1`)
- No ANSI codes when `NO_COLOR=1`
- Double-space column separator

**Milestone:** `cargo test -p oj` passes with table_tests.

---

### Phase 2: Migrate `oj pipeline list`

Replace `format_pipeline_list()` in `pipeline.rs` with the `Table` API.

The pipeline list is the most complex table (conditional PROJECT and RETRIES columns). This migration validates that the `Table` API can handle:

- Conditional columns (only add PROJECT column when `show_project`, only add RETRIES when `show_retries`)
- Muted ID column (`CellStyle::Muted`)
- Status column as last column (`CellStyle::Status`)
- Writing to `&mut impl Write` (pipeline list already does this for testability)

**Before (representative, ~180 lines across 4 conditional branches):**
```rust
if show_project {
    if show_retries {
        writeln!(out, "{} {} {} {} {} {} {} {}", ...);
        for ... { writeln!(out, "{} ..."); }
    } else {
        writeln!(out, "{} {} {} {} {} {} {}", ...);
        for ... { writeln!(out, "{} ..."); }
    }
} else if show_retries {
    ...
} else {
    ...
}
```

**After (~30 lines):**
```rust
let mut table = Table::new({
    let mut cols = vec![Column::muted("ID")];
    if show_project { cols.push(Column::left("PROJECT")); }
    cols.extend([Column::left("NAME"), Column::left("KIND"), Column::left("STEP"), Column::left("UPDATED")]);
    if show_retries { cols.push(Column::left("RETRIES")); }
    cols.push(Column::status("STATUS"));
    cols
});
for (id, p, updated) in &rows {
    let mut cells = vec![id.to_string()];
    if show_project { cells.push(proj.to_string()); }
    cells.extend([p.name.clone(), p.kind.clone(), p.step.clone(), updated.clone()]);
    if show_retries { cells.push(p.retry_count.to_string()); }
    cells.push(p.step_status.clone());
    table.row(cells);
}
table.render(out);
```

**Update `pipeline_tests.rs`:** existing tests (`list_columns_fit_data`, `list_with_project_column`, etc.) should continue to pass with minimal assertion updates. The column separator changes from single-space to double-space, so assertions matching exact padding may need adjustment.

**Milestone:** `cargo test -p oj` passes. `oj pipeline list` output looks identical (modulo separator width).

---

### Phase 3: Migrate `oj queue list` and `oj queue items`

This is the primary fix. Replace tab-separated output with the `Table` renderer.

**`queue list` changes:**
- Columns: PROJECT (left), NAME (left), TYPE (left), ITEMS (right), WORKERS (left)
- Remove `\t` separators
- Remove `items=` / `workers=` prefixes from data cells — the column header makes them redundant

**`queue items` changes:**
- Columns: ID (muted), STATUS (status), WORKER (left), DATA (left)
- Remove `\t` separators
- Remove `worker=` prefix from data cells

**Milestone:** `oj queue list` and `oj queue items` produce properly aligned output. `cargo test -p oj` passes.

---

### Phase 4: Migrate remaining list commands

Migrate in order of complexity:

1. **`oj project list`** — straightforward, already well-structured. Columns: NAME (left), ROOT (left), PIPELINES (right), WORKERS (right), AGENTS (right), CRONS (right).

2. **`oj cron list`** — conditional PROJECT column, same pattern as pipeline. Columns: KIND (left), [PROJECT (left)], INTERVAL (left), PIPELINE (left), TIME (left), STATUS (status).

3. **`oj worker list`** — conditional PROJECT column. Columns: NAME (left), [PROJECT (left)], QUEUE (left), STATUS (status), ACTIVE (left), CONCURRENCY (left). Removes the string-slicing truncation (`&w.name[..w.name.len().min(name_w)]`) — the table renderer handles width internally.

4. **`oj agent list`** — switches from fixed widths to dynamic widths. Columns: ID (muted, max_width=8), NAME (left), PROJECT (left), PIPELINE (left, max_width=8), STEP (left), STATUS (status), READ (right), WRITE (right), CMDS (right). Uses `max_width` on Column instead of manual `truncate()` calls.

**Milestone:** All 7 list commands use the `Table` renderer. `cargo test -p oj` passes.

---

### Phase 5: Clean up dead code and finalize

1. Remove the standalone `truncate()` helper from `agent.rs` / `pipeline.rs` if no longer used.
2. Remove `format_pipeline_list()` as a standalone function — it can become a thin wrapper or be inlined into `handle()` (keeping the `&mut impl Write` signature for testability, possibly by having `Table::render` return a `String` or accept `&mut impl Write`).
3. Verify `cargo clippy --all -- -D warnings` passes.
4. Run `make check` to confirm everything is clean.

**Milestone:** No dead code. `make check` passes.

## Key Implementation Details

### Color-safe padding

The central design decision is: **pad first, colorize second, concatenate with literal separators**. Each cell is rendered independently as `pad(text, width, align)` → `colorize(padded_text, style)` → join all cells with `"  "`. This avoids the current problem where `println!("{:<w$}", colored_string)` sees ANSI bytes in the width calculation.

### Column separator

Use `"  "` (double space) as the column separator. This matches the existing `oj project list` convention and provides clear visual separation. Single space (as in pipeline/cron/worker) is too tight; tabs are unpredictable. Double space is a good middle ground.

### Conditional columns

Columns are added dynamically via `Vec<Column>`, and rows push cells in the same order. The table doesn't need to know about "optional" columns — callers simply include or exclude columns and their corresponding cells.

### Write target

`Table::render` should accept `&mut impl Write` (from `std::fmt::Write`) to support both:
- Direct `stdout` via a wrapper: `&mut StdoutWriter` (or just return `String` and let caller print)
- `&mut Vec<u8>` / `&mut String` for tests

The simplest approach: `Table::render` returns a `String`, and callers do `print!("{}", table.render())`. Or `table.render(&mut out)` where `out: &mut impl std::fmt::Write`. The existing `format_pipeline_list` uses `std::io::Write`; we should match that for consistency.

Use `std::io::Write` so callers can pass `&mut std::io::stdout()` or `&mut Vec<u8>` for tests.

### No external dependencies

The renderer is ~100 lines of straightforward Rust. Adding a crate like `comfy-table` or `tabled` would be overkill for this use case and adds dependency weight to a CLI binary.

## Verification Plan

1. **Unit tests (`table_tests.rs`)**: Cover width calculation, alignment, truncation, color application, edge cases (empty table, single column, very long values).

2. **Existing tests pass**: `pipeline_tests.rs` tests for `format_pipeline_list` must still pass (with minor assertion updates for separator changes).

3. **Manual verification**: Run each list command with sample data and visually confirm alignment:
   - `oj pipeline list`
   - `oj queue list` / `oj queue items`
   - `oj cron list`
   - `oj worker list`
   - `oj agent list`
   - `oj project list`

4. **Color verification**: Run with `COLOR=1` and `NO_COLOR=1` to verify color toggle works.

5. **`make check`**: Full CI verification — fmt, clippy, build, test, deny.
