# Macro-Generated Boilerplate

Several patterns repeat across the codebase with slight variations: builder
structs with identical setter methods, Display impls that map enum variants
to string literals, effect/event dispatch methods with exhaustive match
arms, and executor effect handlers that each follow the same
extract-execute-emit structure. The codebase already uses macros
effectively in places (`define_id!` in core, `lex_tests!` in shell) but
underutilizes them elsewhere. Extending the macro approach to builders,
Display, and dispatch would eliminate ~1,500-2,000 lines of repetitive
source code.

The effect executor is the most impactful target: 14 match arms each
following the same structure (destructure effect, call adapter, emit
event, return). A trait-based handler registry or macro-generated dispatch
would compress this substantially. Similarly, 4+ builder structs implement
identical `fn field(mut self, v: impl Into<T>) -> Self` chains, and 8+
enums implement Display by mapping each variant to a string literal. The
timer handler routes via string prefix stripping on timer IDs, which could
be replaced by a parsed `TimerKind` enum.

- `crates/engine/src/executor.rs` (747 lines) -- 14-arm effect dispatch, each 30-50 lines
- `crates/core/src/effect.rs` (278 lines) -- Effect enum with `name()`, `fields()`, `verbose()` dispatch methods
- `crates/core/src/event/dispatch.rs` (212 lines) -- Event enum with `name()`, `log_summary()` dispatch
- `crates/core/src/agent_run.rs` (246 lines) -- AgentRunBuilder with 8 identical setter methods
- `crates/core/src/job.rs` (618 lines) -- JobConfigBuilder with 6 identical setter methods
- `crates/engine/src/decision_builder.rs` (386 lines) -- EscalationDecisionBuilder with setter chain
- `crates/adapters/src/agent/mod.rs` (238 lines) -- AgentSpawnConfig with 8 setter methods
- `crates/core/src/id.rs` (155 lines) -- existing `define_id!` macro (good pattern to extend)
- `crates/engine/src/runtime/handlers/timer.rs` (656 lines) -- string prefix routing for 10 timer types

## Acceptance Criteria

- A `builder!` macro (or similar) generates setter methods for AgentRunBuilder, JobConfigBuilder, AgentSpawnConfig, and EscalationDecisionBuilder; each builder definition fits in ~15-20 lines
- A `simple_display!` macro (or `strum::Display` derive) replaces manual Display impls for enums that map variants to string literals
- Effect `name()`, `fields()`, and `verbose()` methods are macro-generated or table-driven instead of exhaustive match blocks
- The executor dispatches effects via a trait-based handler or macro-generated match, with each handler isolated in its own function/impl
- No regressions in executor behavior; all effect-related tests pass
- All existing tests pass; `make check` is green
- Net reduction of at least 1,000 lines of source code
