# Test Compression

Tests account for ~64,000 lines (61% of the codebase). The volume is
driven by three structural issues: example-based tests where parameterized
or property tests would be more concise, duplicated test fixtures across
sibling modules, and existing macro infrastructure that some test files
don't use. The state machines are pure (no I/O, deterministic via
FakeClock), making them ideal for property-based testing, but zero property
tests exist today. The engine runtime_tests alone have a 4:1
setup-to-assertion ratio.

The shell crate has the clearest quick win: 5 lexer test files
(variables.rs, substitution.rs, errors.rs, nesting.rs, expansions.rs)
don't use the `lex_tests!` macro defined in the same directory, despite 7
sibling files using it successfully. Converting these would save ~1,200
lines with minimal risk. In the daemon, the mutation and query test modules
define nearly identical fixture builders (`make_job`, `make_breadcrumb`,
`make_worker`) independently. In the engine, the runtime_tests and handler
unit tests overlap significantly, testing the same state transitions at two
layers. Across the engine and daemon, groups of near-identical tests vary
only by input data and could use `yare::parameterized` (already a project
dependency).

- `crates/shell/src/lexer_tests/variables.rs` -- 930 lines, 69 tests, does not use lex_tests! macro
- `crates/shell/src/lexer_tests/substitution.rs` -- 622 lines, does not use lex_tests! macro
- `crates/shell/src/lexer_tests/errors.rs` -- 337 lines, does not use lex_tests! macro
- `crates/shell/src/lexer_tests/nesting.rs` -- 335 lines, does not use lex_tests! macro
- `crates/shell/src/lexer_tests/expansions.rs` -- 139 lines, does not use lex_tests! macro
- `crates/shell/src/lexer_tests/macros.rs` (111 lines) -- existing lex_tests!/lex_error_tests! macros
- `crates/engine/src/runtime_tests/mod.rs` -- test harness with ~2,000 lines of infrastructure
- `crates/engine/src/runtime_tests/job_create.rs` -- 991 lines, 24 tests, repetitive workspace setup
- `crates/engine/src/spawn_tests.rs` (884 lines) -- 24 tests, variable interpolation groups parameterizable
- `crates/engine/src/decision_builder_tests.rs` (774 lines) -- 25 tests, repeated match/assert boilerplate
- `crates/daemon/src/listener/mutations/test_helpers.rs` -- fixture builders for mutation tests
- `crates/daemon/src/listener/query_tests/mod.rs` -- parallel fixture builders for query tests
- `crates/daemon/src/listener/decisions_tests.rs` (914 lines) -- 25+ tests with identical match boilerplate
- `crates/shell/tests/e2e_tests.rs` (764 lines) -- 34 tests, sequential operation groups parameterizable
- `crates/runbook/src/find_tests.rs` (955 lines) -- similar test patterns across find scenarios
- `crates/runbook/src/command_tests.rs` (795 lines) -- repetitive CommandDef construction

## Acceptance Criteria

- All 5 shell lexer test files (variables, substitution, errors, nesting, expansions) use the existing `lex_tests!` / `lex_error_tests!` macros
- Daemon mutation and query test modules share a single set of fixture builders instead of defining parallel copies
- At least 3 test files in engine or daemon use `yare::parameterized` to collapse groups of near-identical tests
- Engine runtime_tests and handler unit tests have no redundant coverage of the same state transition at both layers
- Decision test files use assertion helpers (e.g., `assert_decision_created()`) instead of repeating match/panic boilerplate
- All existing tests pass; `make check` is green; test coverage does not decrease
- Net reduction of at least 4,000 test lines
