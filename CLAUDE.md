# Oddjobs (oj)

An agentic development team to write software and automate other tasks.

## Architecture Overview

The system is a user-level daemon (`ojd`) that executes pipelines defined in HCL (or TOML) runbooks (`.oj/runbooks/*.hcl`). The CLI (`oj`) communicates with the daemon over a Unix socket. Pipelines have steps that run either shell commands or agents (Claude Code in tmux sessions). State is durably stored via a write-ahead log (WAL) with periodic snapshots, allowing recovery across daemon restarts. The architecture follows a functional core / imperative shell pattern: pure state machines in `crates/core`, effects as data, and trait-based adapters (`SessionAdapter`, `RepoAdapter`, `AgentAdapter`) in `crates/adapters` for testability.

Agents are interactive entities defined in runbooks. Each agent has its own configuration (command, prompt, lifecycle handlers, notifications) and runs in an isolated tmux session. Some agents are short-lived, completing a single task and exiting; others are long-lived, persisting across multiple interactions. Pipelines are the primary way agents are triggered today, but agent definitions are standalone — an agent's identity, lifecycle, and notify config are independent of the pipeline that spawns it.

Agent lifecycle is managed by per-agent file watchers that monitor Claude's JSONL session log for state changes (working/idle/failed/exited). Agents run in isolated git worktrees (`workspace = "ephemeral"`) and their lifecycle is handled by configurable actions: `on_idle` (when agent is waiting for input) supports `nudge`, `done`, `fail`, `escalate`, and `gate`; `on_dead` (when agent exits) supports `done`, `fail`, `recover`, `escalate`, and `gate`. The `gate` action runs a shell command — exit 0 advances the pipeline, non-zero escalates. Both pipelines and agents support `notify {}` blocks with `on_start`, `on_done`, and `on_fail` message templates that emit desktop notifications on lifecycle events.

A single daemon serves all projects for a user. Per-project namespace isolation prevents resource collisions: pipelines, workers, and queues are scoped by a namespace derived from `.oj/config.toml [project].name` (falling back to the directory basename). Namespaces propagate through events, IPC requests, and the `OJ_NAMESPACE` environment variable so that nested `oj` calls from agents inherit the parent project's context. Queues support dead letter semantics with configurable retry — failed items are automatically retried with cooldown, and items that exhaust retries move to `Dead` status. Dead items can be resurrected with `oj queue retry`.

### Why Agents Run in tmux (Not Print Mode)

Agents are long-lived and interactive by design.
The tmux-based architecture enables:
- **Observability**: Users and other agents can attach to sessions to monitor work in real-time
- **Intervention**: Users and other agents can communicate with running agents when needed
- **Debugging**: Interactive access to diagnose and fix issues

NEVER using `claude -p` (print mode) in runbooks.
ONLY use the flag in tests when EXPLICITLY testing the `on_dead` or `--print` handling.

Instead:
- Use `on_idle` as a safety net for agents who don't notify when they're done.
- Use `on_dead` as a safety net for **unexpected** process termination.

## Directory Structure

```toc
oddjobs/
├── crates/           # Rust workspace
│   ├── adapters/     # Trait implementations (AgentAdapter, RepoAdapter, etc.)
│   ├── cli/          # Command-line interface (oj)
│   ├── core/         # Core library (state machines, effects)
│   ├── daemon/       # User-level daemon (ojd)
│   ├── engine/       # Pipeline execution engine
│   ├── runbook/      # HCL/TOML runbook parsing
│   ├── shell/        # Shell command execution
│   └── storage/      # WAL and snapshot persistence
├── docs/             # Architecture documentation
├── plans/            # Epic implementation plans
├── scripts/          # Build and utility scripts
└── tests/            # End-to-end tests
```

## Development Policies

### NO ARBITRARY SLEEPS
**NEVER use `sleep()`, `thread::sleep()`, or `tokio::time::sleep()` as a synchronization mechanism.**
- Sleeps hide race conditions instead of fixing them
- Sleeps make tests slow and flaky
- Use proper synchronization: channels, condition variables, or polling with backoff
- If you need to wait for state, poll for the actual condition with a timeout
- Exception: heartbeat intervals or intentional rate limiting (not synchronization)

### Dead Code Policy
- All unused code must be removed, not commented out
- Unused dependencies must be removed from Cargo.toml

### Test Conventions
- Unit tests in `*_tests.rs` files, imported from the module under test:
  ```rust
  // In protocol.rs:
  #[cfg(test)]
  #[path = "protocol_tests.rs"]
  mod tests;
  ```
- Integration tests in `tests/` directory
- Use `FakeClock`, `FakeAdapters` for deterministic tests
- Property tests for state machine transitions
- Coverage reports via `scripts/coverage` (uses llvm-cov)

### API Surface Policy
- Proactively avoid unnecessary exports or re-exports
- Minimize the public API surface of modules
- Expect future changes to add exports as needed rather than exporting speculatively

## Commits

Use conventional commit format: `type(scope): description`
Types: feat, fix, chore, docs, test, refactor

## Landing the Plane

Before committing changes:

- [ ] Run `make check` for full verification
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `quench check` (must pass)
  - `cargo test --all`
  - `cargo build --all`
  - `cargo audit`
  - `cargo deny check licenses bans sources`
