# Plan: Attach to Agent on `oj run`

## Overview

Add the ability for `oj run` to automatically attach the user's terminal to the agent's tmux session when a command dispatches a standalone agent (`run = { agent = "..." }`). This is controlled by a new `attach` field in the `RunDirective::Agent` variant and overridable via `--attach` / `--no-attach` CLI flags. Attaching is a no-op when stdout is not a TTY.

## Project Structure

Key files to modify:

```
crates/runbook/src/command.rs          # Add `attach` field to RunDirective::Agent
crates/runbook/src/command_tests.rs    # Tests for new field deserialization
crates/cli/src/commands/run.rs         # Add CLI flags, attach-after-dispatch logic
crates/daemon/src/protocol.rs          # Return session_id in AgentRunStarted response
crates/daemon/src/listener/commands.rs # Populate session_id in response (or not — see phase 3)
~/Developer/.oj/runbooks/mayor.hcl     # Set attach = true on mayor command
```

## Dependencies

No new external dependencies. Uses existing:
- `std::io::IsTerminal` (stable since Rust 1.70) for TTY detection
- `clap` for CLI flag parsing
- `tokio::signal::ctrl_c` for interruptible polling (already used in `dispatch_pipeline`)

## Implementation Phases

### Phase 1: Add `attach` field to `RunDirective::Agent`

**Goal:** Parse `attach = true` from the `run` block in command definitions.

**Files:** `crates/runbook/src/command.rs`, `crates/runbook/src/command_tests.rs`

Expand `RunDirective::Agent` to include an optional `attach` field:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RunDirective {
    Shell(String),
    Pipeline { pipeline: String },
    Agent {
        agent: String,
        #[serde(default)]
        attach: Option<bool>,
    },
}
```

Add a helper method:

```rust
impl RunDirective {
    pub fn attach(&self) -> Option<bool> {
        match self {
            RunDirective::Agent { attach, .. } => *attach,
            _ => None,
        }
    }
}
```

HCL usage:

```hcl
command "mayor" {
  run = { agent = "mayor", attach = true }
}
```

Add tests to `command_tests.rs`:
- Deserializing `run = { agent = "x", attach = true }` yields `RunDirective::Agent { agent: "x", attach: Some(true) }`
- Deserializing `run = { agent = "x" }` yields `attach: None` (backward compatible)
- The `attach()` accessor returns the correct values

**Milestone:** `cargo test -p oj-runbook` passes with new tests.

---

### Phase 2: Add `--attach` / `--no-attach` CLI flags to `oj run`

**Goal:** Let users override the runbook's attach setting from the command line.

**File:** `crates/cli/src/commands/run.rs`

Add flags to `RunArgs`:

```rust
#[derive(Args)]
pub struct RunArgs {
    pub command: Option<String>,
    #[arg(trailing_var_arg = true)]
    pub args: Vec<String>,
    #[arg(long = "runbook")]
    pub runbook: Option<String>,

    /// Attach to the agent's tmux session after starting
    #[arg(long = "attach", conflicts_with = "no_attach")]
    pub attach: bool,
    /// Do not attach to the agent's tmux session
    #[arg(long = "no-attach", conflicts_with = "attach")]
    pub no_attach: bool,
}
```

Compute effective attach preference in `handle()`:

```rust
let attach_override = if args.attach {
    Some(true)
} else if args.no_attach {
    Some(false)
} else {
    None
};
```

Pass this through to `dispatch_to_daemon` and `dispatch_agent_run`. The final decision is:

```rust
// In dispatch_agent_run:
let should_attach = attach_override
    .or(runbook_attach)   // from RunDirective::Agent { attach, .. }
    .unwrap_or(false);    // default: don't attach
```

The `--attach` and `--no-attach` flags must not be confused with command-level flags defined in the runbook's `args` spec. Since they are defined in `RunArgs` as clap `#[arg(long)]`, clap will consume them before `trailing_var_arg` processes the remaining args. However, because `trailing_var_arg = true` is greedy, these flags need careful ordering. The flags should be parsed by clap natively — they sit outside the trailing var args.

**Important:** `trailing_var_arg = true` means clap stops parsing its own flags after the first positional arg. This means `oj run mycommand --attach` would pass `--attach` as a trailing arg, not a clap flag. To handle this correctly, either:
1. Filter `--attach`/`--no-attach` from `args.args` before passing them to the runbook arg parser, or
2. Require flags before the command name: `oj run --attach mycommand`, or
3. Remove `trailing_var_arg` and use a different approach.

**Recommended approach:** Manually scan and remove `--attach`/`--no-attach` from `args.args` in the `handle()` function before forwarding to the runbook arg splitter, setting the boolean accordingly. This allows `oj run fix "some bug" --attach` to work naturally.

```rust
// At the top of handle(), before prevalidate:
let mut raw_args = args.args.clone();
let mut cli_attach = None;
raw_args.retain(|a| {
    if a == "--attach" {
        cli_attach = Some(true);
        false
    } else if a == "--no-attach" {
        cli_attach = Some(false);
        false
    } else {
        true
    }
});
let attach_override = if args.attach { Some(true) }
    else if args.no_attach { Some(false) }
    else { cli_attach };
```

**Milestone:** `oj run --help` shows `--attach` and `--no-attach` flags.

---

### Phase 3: Implement attach-after-dispatch in the CLI

**Goal:** After dispatching a standalone agent to the daemon, poll for the session to become available and attach to it.

**File:** `crates/cli/src/commands/run.rs`

Convert `dispatch_agent_run` from a sync function to async. Follow the same polling pattern as `dispatch_pipeline` (lines 233-285):

```rust
async fn dispatch_agent_run(
    client: &DaemonClient,
    namespace: &str,
    command: &str,
    agent_run_id: &str,
    agent_name: &str,
    should_attach: bool,
) -> Result<()> {
    let short_id = &agent_run_id[..8.min(agent_run_id.len())];
    println!("Project: {namespace}");
    println!("Command {command} invoked.");
    println!("Agent: {agent_name} ({short_id})");
    println!();

    if !should_attach || !std::io::stdout().is_terminal() {
        // No attach: print helper commands and return
        println!("  oj agent show {short_id}");
        println!("  oj agent logs {short_id}");
        return Ok(());
    }

    // Poll for session_id to appear on the agent run
    println!("Waiting for agent session... (Ctrl+C to skip)");

    let poll_interval = Duration::from_millis(300);
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut session_id = None;

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = &mut ctrl_c => break,
            _ = tokio::time::sleep(poll_interval) => {
                if Instant::now() >= deadline { break; }
                if let Ok(Some(detail)) = client.get_agent(agent_run_id).await {
                    if let Some(ref sid) = detail.session_id {
                        session_id = Some(sid.clone());
                        break;
                    }
                }
            }
        }
    }

    match session_id {
        Some(sid) => {
            // Attach to tmux session (blocks until user detaches)
            crate::commands::session::attach(&sid)?;
        }
        None => {
            println!("Agent session not ready yet. You can attach manually:");
            println!("  oj attach {short_id}");
        }
    }

    Ok(())
}
```

Key details:
- **TTY check:** `std::io::stdout().is_terminal()` — if not a TTY, skip attach silently.
- **Polling target:** `client.get_agent(agent_run_id)` — the `GetAgent` query handler in `crates/daemon/src/listener/query.rs:216-251` matches standalone agent runs by `ar.id` (which equals the `agent_run_id`). The returned `AgentDetail.session_id` is populated from `ar.session_id` once the tmux session is spawned.
- **Ctrl+C:** Interrupting the poll skips attachment but leaves the agent running.
- **Detaching from tmux:** Standard tmux behavior — `Ctrl+B D` detaches, leaving the agent running.

Update `dispatch_to_daemon` to pass through the attach preference and the runbook's `attach` value:

```rust
async fn dispatch_to_daemon(
    client: &DaemonClient,
    project_root: &Path,
    invoke_dir: &Path,
    namespace: &str,
    command: &str,
    positional: &[String],
    named: &HashMap<String, String>,
    runbook_attach: Option<bool>,  // from RunDirective::Agent { attach }
    attach_override: Option<bool>, // from CLI flags
) -> Result<()> {
    // ... existing dispatch logic ...

    match result {
        RunCommandResult::AgentRun { agent_run_id, agent_name } => {
            let should_attach = attach_override
                .or(runbook_attach)
                .unwrap_or(false);
            dispatch_agent_run(
                client, namespace, command,
                &agent_run_id, &agent_name,
                should_attach,
            ).await
        }
        // Pipeline case unchanged
        // ...
    }
}
```

**Milestone:** `oj run mayor --attach` polls for and attaches to the agent's tmux session.

---

### Phase 4: Update the mayor runbook

**Goal:** Set `attach = true` in the mayor command's run block.

**File:** `~/Developer/.oj/runbooks/mayor.hcl`

Change the mayor command's `run` block:

```hcl
# Before:
command "mayor" {
  run = { agent = "mayor" }
}

# After:
command "mayor" {
  run = { agent = "mayor", attach = true }
}
```

**Milestone:** Running `oj run mayor` automatically attaches to the mayor's tmux session.

---

### Phase 5: Tests

**Goal:** Verify the full feature works correctly.

**Files:** `crates/runbook/src/command_tests.rs`, `crates/cli/src/commands/run_tests.rs`

Tests to add:

1. **Runbook parsing tests** (`command_tests.rs`):
   - `deserialize_agent_run_with_attach_true` — `{ agent = "x", attach = true }` parses correctly
   - `deserialize_agent_run_with_attach_false` — `{ agent = "x", attach = false }` parses correctly
   - `deserialize_agent_run_without_attach` — `{ agent = "x" }` has `attach: None`
   - `attach_accessor_returns_none_for_non_agent` — `RunDirective::Shell` and `Pipeline` return `None`

2. **CLI flag tests** (`run_tests.rs`):
   - Verify `--attach` and `--no-attach` are parsed from the args correctly
   - Verify precedence: CLI override > runbook default > false

**Milestone:** `cargo test --all` passes.

## Key Implementation Details

### Attach decision precedence

```
CLI --attach/--no-attach  >  runbook attach = true/false  >  default (false)
```

### TTY guard

Attach is silently skipped when stdout is not a terminal. This prevents breakage in scripts, CI, or piped output.

### Session ID availability

The agent's `session_id` is `None` until the daemon spawns the tmux session. The timeline is:
1. CLI sends `RunCommand` request
2. Daemon returns `AgentRunStarted { agent_run_id, agent_name }`
3. Engine spawns agent: generates `agent_id` (UUID), creates tmux session `oj-<agent_id>`, sets `session_id` on `AgentRun`
4. CLI polls `GetAgent(agent_run_id)` until `session_id` is `Some`

Typical spawn latency is under 1 second, so the 15-second deadline is generous.

### `serde(untagged)` ordering

The `RunDirective` enum uses `#[serde(untagged)]`. The variant order matters: `Shell(String)` must come first (matches plain strings), then `Pipeline { pipeline }`, then `Agent { agent, attach }`. Adding `attach: Option<bool>` with `#[serde(default)]` to the `Agent` variant preserves this ordering — if the input has an `agent` key, it matches `Agent`; if it has a `pipeline` key, it matches `Pipeline`.

## Verification Plan

1. **Unit tests:** `cargo test -p oj-runbook` — new deserialization tests
2. **Lint/format:** `cargo fmt --all && cargo clippy --all -- -D warnings`
3. **Full build:** `cargo build --all`
4. **Full test suite:** `cargo test --all`
5. **Manual smoke test:**
   - `oj run mayor` — should auto-attach (after mayor runbook update)
   - `oj run mayor --no-attach` — should print helper commands, no attach
   - `oj run build x y` — pipeline command, no change in behavior
   - `oj run mayor | cat` — piped stdout, attach silently skipped
6. **CI:** `make check` passes
