# Spec Rules

These are behavioral specifications for oj. They test the CLI as a black box.

## Golden Rule

**Specs test behavior, not implementation.**

Write specs by reading `docs/`, not by reading `src/`.

## Performance Budget

Tests must be fast. File a performance bug if these limits can't be met.

| Metric | Limit |
|--------|-------|
| Avg passing test | < 100ms |
| Single passing test | < 350ms |
| Single failing test (timeout) | ~2000ms |
| Full spec suite | < 5s |

## DO

- Use `cli().args(&[...]).passes()` for CLI tests
- Use `Project::empty()` with `git_init()` and `file()` for setup
- Check stdout, stderr, and exit codes
- Use `#[ignore = "TODO: description"]` for unimplemented specs
- Use `wait_for(SPEC_WAIT_MAX_MS, || condition)` for async checks
- Use `SPEC_*` constants for all timeouts

## DO NOT

- Import anything from `oj::*` or `oj_*::*`
- Read or inspect internal state
- Call internal functions directly
- Write specs by looking at the implementation
- **Use `std::thread::sleep`** - use `wait_for` instead
- **Use magic numbers** - define or use `SPEC_*` constants

## Helpers Available

```rust
use crate::prelude::*;

// CLI builder
cli().args(&["daemon", "status"]).passes();
cli().args(&["--help"]).passes().stdout_has("Usage:");
cli().pwd("/tmp/project").args(&["daemon", "start"]).passes();

// Project helper (auto-cleans up daemon on drop)
let temp = Project::empty();
temp.git_init();
temp.file(".oj/runbooks/build.toml", MINIMAL_RUNBOOK);
temp.oj().args(&["daemon", "start"]).passes();
temp.oj().args(&["daemon", "status"]).passes().stdout_has("running");

// Output assertions
.passes()           // expect exit 0
.fails()            // expect non-zero exit
.stdout_eq("x")     // exact match (preferred - with diff on failure)
.stdout_has("x")    // contains (when exact comparison isn't practical)
.stdout_lacks("x")  // doesn't contain
.stderr_has("x")    // stderr contains

// Polling for async conditions (NO SLEEPS)
let ready = wait_for(SPEC_WAIT_MAX_MS, || {
    temp.oj().args(&["job", "list"]).passes().stdout().contains("Done")
});
assert!(ready, "job should complete");
```

## Constants

```rust
// Defined in prelude.rs
pub const SPEC_POLL_INTERVAL_MS: u64 = 10;   // Polling frequency
pub const SPEC_WAIT_MAX_MS: u64 = 2000;      // Max wait for async conditions
```

## Output Comparison

**Prefer exact comparison** - catches format regressions and unexpected changes:

```rust
// BEST: Exact output comparison with diff on failure
cli().args(&["--version"]).passes().stdout_eq("oj 0.1.0\n");

// ACCEPTABLE: Pattern matching when exact comparison isn't practical
temp.oj().args(&["daemon", "status"]).passes().stdout_has("Status: running");

// AVOID: Vague checks that miss format regressions
temp.oj().args(&["daemon", "status"]);  // No output validation at all
```

**When to use each:**
- `stdout_eq(expected)` - **Default choice.** Use for format specs and stable output
- `stdout_has(pattern)` - When output varies (timestamps, counts, dynamic IDs)
- `stdout_lacks(pattern)` - Verify absence (no debug output, no errors)

## Running Specs

```bash
cargo test --test specs              # All specs
cargo test --test specs -- --ignored # Show unimplemented count
cargo test --test specs cli_help     # Just help tests
```

## Claude Modes

**`claude -p` (and `claudeless -p`) means `--print` (non-interactive).** This is critical for writing agent tests:

| Mode | Flag | Behavior | Lifecycle handler |
|------|------|----------|-------------------|
| **Print** | `-p 'prompt'` | Responds once and **exits** | `on_dead` |
| **Interactive** | `'prompt'` (no `-p`) | Responds and **stays alive**, waiting for input | `on_idle` |

- **`-p` tests**: Agent exits immediately after one response. The watcher detects session death and fires `on_dead`. Use `on_dead = "done"` to advance the job.
- **Interactive tests**: Agent stays alive and idles. The watcher detects idleness agent and fires `on_idle`. Use `on_idle = "done"` to advance the job.

**Common mistake:** Using `on_idle = "done"` with `-p` mode. The agent exits before idling, so `on_idle` never fires. The job gets stuck because `on_dead` defaults to `"escalate"`.

## Debugging

Each test uses an isolated state directory via `OJ_STATE_DIR`. Key log files:

| File | Purpose |
|------|---------|
| `daemon.log` | Daemon startup, request handling, engine events |
| `cli.log` | CLI errors (connection failures, missing context) |
| `daemon.pid` | Daemon process ID |
| `daemon.sock` | Unix socket for CLI-daemon IPC |

**Finding logs for a failed test:**

```rust
// In your test, print the state dir path on failure
let temp = Project::empty();
// ... test code ...
eprintln!("State dir: {:?}", temp.state_dir());
```

Or check the test's temp directory (printed by cargo test on failure).

**What happens without `OJ_STATE_DIR`:**

When the CLI runs without `OJ_STATE_DIR` set, it falls back to `~/.local/state/oj` (the user's real daemon). This means:
- Spawned agents will talk to the **wrong daemon** if `OJ_STATE_DIR` isn't passed through
- The job appears stuck even though the agent "completed"

Check `~/.local/state/oj/cli.log` (not the test's cli.log) for evidence - it logs `OJ_STATE_DIR=(not set)` and the socket path used.

The fix: `spawn.rs` passes `OJ_STATE_DIR` to spawned sessions via the env list.

**Common debugging scenarios:**

1. **Agent spawn issues** - Check `daemon.log` for spawn effects and session events
2. **Agent not completing** - Check `daemon.log` for on_dead/on_idle action handling
3. **Daemon not starting** - Check `daemon.log` for startup errors after the marker line
4. **Job stuck** - Check `daemon.log` for the request log: `received request`
5. **Agent completes but job stuck** - Check if `OJ_STATE_DIR` was passed to the session

**Tmux sessions (agent tests):**

```bash
tmux list-sessions           # See active sessions
tmux attach -t <session>     # Attach to inspect
tmux capture-pane -p -t <session>  # Dump pane contents
```
