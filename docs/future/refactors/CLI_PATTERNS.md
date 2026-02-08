# CLI Pattern Extraction

The CLI crate (8,650 source lines) has repetitive structural patterns
across its command modules. Each entity command (job, agent, session,
worker, queue, cron, decision) follows the same Args-to-Enum-to-match-to-
handler structure, defines similar list-formatting functions with
conditional project columns, and branches on text vs JSON output format
20+ times. Extracting shared abstractions for table formatting, output
format dispatch, and list filtering would reduce the CLI by ~600-800 lines
while making new commands cheaper to add.

Six `format_X_list()` functions build tables with the same column setup,
namespace filtering, sort, and truncation logic. Every command handler
repeats `match format { Text => ..., Json => ... }`. The list commands
each independently implement filter-by-project, filter-by-status,
sort-by-field, and apply-limit. A `TableBuilder` with a `FilteredList`
helper and a `Formatter` trait would collapse these patterns into shared
infrastructure.

- `crates/cli/src/commands/job.rs` -- largest command module; list formatting, filter/sort, format branching
- `crates/cli/src/commands/queue.rs` -- repeats table formatting and filter patterns from job.rs
- `crates/cli/src/commands/decision.rs` -- repeats table formatting pattern
- `crates/cli/src/commands/session.rs` -- repeats list formatting pattern
- `crates/cli/src/commands/worker.rs` -- repeats start/stop/list pattern from cron
- `crates/cli/src/commands/agent/` -- agent display with repeated formatting
- `crates/cli/src/commands/status.rs` -- status overview with format branching
- `crates/cli/src/commands/mod.rs` -- top-level command dispatch

## Acceptance Criteria

- A shared `TableBuilder` or equivalent handles column definition, namespace-conditional columns, and rendering for all entity list commands
- A shared `FilteredList` or equivalent handles filter-by-project, filter-by-status, sort, and limit logic across all list commands
- Text vs JSON output branching is handled by a `Formatter` trait or helper function, not inlined per command
- Adding a new entity list command requires only defining columns and a row mapper, not reimplementing filter/sort/format logic
- All existing tests pass; `make check` is green
- Net reduction of at least 400 lines in the CLI crate
