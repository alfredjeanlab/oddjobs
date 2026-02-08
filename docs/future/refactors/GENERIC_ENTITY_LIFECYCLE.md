# Generic Entity Lifecycle

Jobs, agents, workers, crons, and queues all follow the same lifecycle
pattern (create, start, idle/fail/escalate, done/cancel, prune) but each
has its own bespoke handler implementation, listener mutations, query
transformations, CLI commands, and tests. This multiplier is the single
largest contributor to codebase volume. A generic lifecycle trait would let
entities declare their state transitions and lifecycle hooks, with shared
handler logic driving the common patterns and entity-specific overrides
handling the differences.

The runtime handler directory has parallel files for each entity type that
repeat the same event-to-state-to-effect sequences. The listener mutations
follow the same lock-validate-emit-respond pattern per entity. The test
suites are the most expensive symptom: the runtime_tests suite alone is
~12,000 lines with a 4:1 setup-to-assertion ratio, and the same behaviors
(shell exit handling, timer start, idle timeout, escalation) are tested
separately for each entity type rather than parameterized across them.
Property-based testing would be a natural fit since the state machines are
pure, but zero property tests exist today.

- `crates/engine/src/runtime/handlers/agent.rs` (546 lines) -- agent state change handling, parallel to lifecycle.rs
- `crates/engine/src/runtime/handlers/lifecycle.rs` (384 lines) -- job lifecycle transitions
- `crates/engine/src/runtime/handlers/timer.rs` (656 lines) -- 10 timer handlers routed via string prefix stripping
- `crates/engine/src/runtime/handlers/` -- worker/ and cron/ subdirectories repeat handler patterns
- `crates/engine/src/runtime_tests/` -- ~12,000 lines; same lifecycle tested per entity type
- `crates/engine/src/runtime_tests/worker.rs` -- worker lifecycle tests parallel to job tests
- `crates/engine/src/runtime_tests/cron.rs` -- cron lifecycle tests parallel to job tests
- `crates/engine/src/runtime_tests/agent_run/` -- standalone agent tests parallel to job-embedded agent tests
- `crates/engine/src/runtime_tests/on_dead.rs` -- on_dead handling tested separately per entity
- `crates/engine/src/runtime_tests/monitoring/` -- monitoring tests repeat across entity types
- `crates/daemon/src/listener/workers.rs` (423 lines) -- start/stop/resize mirrors job mutations
- `crates/daemon/src/listener/crons.rs` (415 lines) -- start/stop mirrors worker mutations
- `crates/daemon/src/listener/mutations/agents.rs` -- agent resolution duplicated 3x across kill/send/resume
- `crates/daemon/src/listener/workers_tests.rs` (627 lines) -- tests parallel to job mutation tests
- `crates/daemon/src/listener/decisions_tests.rs` (914 lines) -- decision builder tests repeat per trigger type

## Acceptance Criteria

- A `Lifecycle` or `EntityHandler` trait exists that Job, Worker, Cron, and AgentRun implement
- Common operations (start, stop, prune, resume) are handled by generic functions parameterized over the trait, not per-entity handler files
- The 3x-duplicated agent resolution logic in `mutations/agents.rs` is factored into a single `resolve_agent_to_session()` function
- Timer routing uses a parsed `TimerKind` enum instead of string prefix matching
- At least one property-based test covers state machine transitions across entity types
- Runtime handler tests for shared lifecycle patterns (idle, dead, escalate) are parameterized across entity types rather than duplicated per type
- All existing tests pass; `make check` is green
- Net reduction of at least 5,000 lines across source and tests
