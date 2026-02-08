# Quick Wins

Small, isolated cleanups that require no architectural changes and can be
done independently of the larger refactors. These are safe to land as
individual commits and help build momentum before tackling structural work.
Several also remove friction that would otherwise complicate the later
refactors (e.g., duplicated timestamp functions would confuse a generic
lifecycle extraction).

None of these items have dependencies on each other or on any of the
strategic refactors. They reduce noise in the codebase and make the
subsequent layers of work cleaner to execute.

- `crates/engine/src/breadcrumb.rs:183-222` -- `format_utc_now()` and `days_to_civil()` duplicated verbatim from `time_fmt.rs:9-46`; replace with `use crate::time_fmt::format_utc_now` (~40 lines)
- `crates/engine/src/activity_logger.rs` -- also calls `format_utc_now`; verify it uses the `time_fmt` copy
- `crates/engine/src/runtime/handlers/cron_types.rs` -- also calls `format_utc_now`; verify it uses the `time_fmt` copy
- `crates/adapters/src/session/noop.rs` -- `NoOpSessionAdapter` is unused outside its own tests; remove with `noop_tests.rs` (~90 lines)
- `crates/adapters/src/notify/noop.rs` -- `NoOpNotifyAdapter` is unused outside its own tests; remove with `noop_tests.rs` (~50 lines)
- `crates/adapters/src/lib.rs` -- re-exports both NoOp adapters; remove from public API
- `crates/shell/src/` -- comment density is 51% (1,683 comment lines in 4,402 source lines); reduce to ~20% by trimming comments that restate obvious code (~400 lines)

## Acceptance Criteria

- `breadcrumb.rs` imports `format_utc_now` from `time_fmt` instead of defining its own copy; `days_to_civil` is removed from breadcrumb.rs
- `NoOpSessionAdapter` and `NoOpNotifyAdapter` are removed from the codebase (source, tests, and re-exports)
- No other crate references the removed NoOp types (confirmed: daemon and engine do not use them)
- Shell crate comment ratio is below 30%
- All existing tests pass; `make check` is green
- Net reduction of at least 500 lines
