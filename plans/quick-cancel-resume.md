# Plan: Add `oj cancel` and `oj resume` as Top-Level Shortcuts

## Overview

Add `oj cancel` and `oj resume` as top-level convenience commands, mirroring the pattern used by existing shortcuts (`oj peek`, `oj attach`, `oj logs`, `oj show`). These commands delegate to the same IPC protocol requests already used by `oj pipeline cancel` and `oj pipeline resume` — no daemon or protocol changes are needed.

## Project Structure

Files to modify:

```
crates/cli/src/
├── main.rs          # Add Cancel/Resume variants to Commands enum + dispatch
├── help.rs          # Add cancel/resume to Actions section
└── main_tests.rs    # Add CLI parsing tests for new commands
```

No new files are created. No changes to `crates/daemon/`, `crates/core/`, or protocol definitions.

## Dependencies

None. All required infrastructure already exists:
- `DaemonClient::pipeline_cancel()` — `crates/cli/src/client_queries.rs:168`
- `DaemonClient::pipeline_resume()` — `crates/cli/src/client_queries.rs:153`
- `Request::PipelineCancel` / `Request::PipelineResume` — `crates/daemon/src/protocol.rs`

## Implementation Phases

### Phase 1: Add `Commands` enum variants

**File:** `crates/cli/src/main.rs` (lines 58–120)

Add two new variants to the `Commands` enum, placed after `Show` (at the end of the convenience shortcuts block):

```rust
/// Cancel one or more running pipelines
Cancel {
    /// Pipeline IDs or names (prefix match)
    #[arg(required = true)]
    ids: Vec<String>,
},
/// Resume monitoring for an escalated pipeline
Resume {
    /// Pipeline ID or name
    id: String,
    /// Message for nudge/recovery (required for agent steps)
    #[arg(short = 'm', long)]
    message: Option<String>,
    /// Pipeline variables to set (can be repeated: --var key=value)
    #[arg(long = "var", value_parser = parse_key_value)]
    var: Vec<(String, String)>,
},
```

Import `parse_key_value` from `commands::pipeline` (it is currently private; make it `pub(crate)`).

**Milestone:** `cargo build --all` succeeds (with exhaustive-match errors in dispatch, addressed next).

### Phase 2: Add dispatch handlers

**File:** `crates/cli/src/main.rs`, inside `match command { ... }` (around line 404–438)

Add handlers next to the existing convenience commands. Both are **action** commands (mutate state → use `DaemonClient::for_action()`).

```rust
Commands::Cancel { ids } => {
    let client = DaemonClient::for_action()?;
    pipeline::handle(
        pipeline::PipelineCommand::Cancel { ids },
        &client,
        &namespace,
        format,
    )
    .await?
}
Commands::Resume { id, message, var } => {
    let client = DaemonClient::for_action()?;
    pipeline::handle(
        pipeline::PipelineCommand::Resume { id, message, var },
        &client,
        &namespace,
        format,
    )
    .await?
}
```

This delegates directly to the existing `pipeline::handle()` function, avoiding any code duplication. The output formatting, error handling, and exit codes are identical to `oj pipeline cancel` / `oj pipeline resume`.

**Milestone:** `cargo build --all` succeeds. `oj cancel --help` and `oj resume --help` produce correct usage text.

### Phase 3: Update help text

**File:** `crates/cli/src/help.rs` (lines 41–67)

Add `cancel` and `resume` to the **Actions** section of the help output:

```rust
pub fn commands() -> String {
    "\
Actions:
  run         Run a command from the runbook
  cancel      Cancel one or more running pipelines
  resume      Resume an escalated pipeline
  status      Show overview of active work across all projects
  show        Show details of a pipeline, agent, session, or queue
  peek        Peek at the active tmux session
  attach      Attach to a tmux session

Resources:
  ..."
        .to_string()
}
```

**Milestone:** `oj --help` shows `cancel` and `resume` in the Actions section.

### Phase 4: Add CLI parsing tests

**File:** `crates/cli/src/main_tests.rs`

Add tests verifying argument parsing for both new commands:

```rust
#[test]
fn cancel_requires_at_least_one_id() {
    let result = cli_command().try_get_matches_from(["oj", "cancel"]);
    assert!(result.is_err());
}

#[test]
fn cancel_accepts_multiple_ids() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "cancel", "abc", "def"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert!(matches!(cli.command, Some(Commands::Cancel { ids }) if ids.len() == 2));
}

#[test]
fn resume_accepts_id_and_message() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "resume", "abc", "-m", "try again"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert!(matches!(cli.command, Some(Commands::Resume { id, message, .. })
        if id == "abc" && message.as_deref() == Some("try again")));
}

#[test]
fn resume_accepts_var_flags() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "resume", "abc", "--var", "key=val"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert!(matches!(cli.command, Some(Commands::Resume { var, .. }) if var.len() == 1));
}
```

**Milestone:** `cargo test --all` passes.

### Phase 5: Final verification

Run `make check`:
- `cargo fmt --all`
- `cargo clippy --all -- -D warnings`
- `cargo build --all`
- `cargo test --all`

Verify end-to-end behavior:
- `oj --help` lists cancel/resume in Actions
- `oj cancel --help` shows correct usage
- `oj resume --help` shows correct usage with `-m` and `--var` options
- `oj cancel` (no args) exits with error (required argument)
- `oj resume` (no args) exits with error (required argument)

## Key Implementation Details

### Delegation, not duplication

The dispatch handlers construct `PipelineCommand::Cancel` / `PipelineCommand::Resume` and delegate to `pipeline::handle()`. This means:
- Output formatting is shared (text and JSON modes both work)
- Error handling is shared (not-found, already-terminal, message-required errors)
- Future improvements to pipeline cancel/resume output automatically apply to shortcuts

### `parse_key_value` visibility

The `--var key=value` parser (`parse_key_value`) is defined in `crates/cli/src/commands/pipeline.rs:124`. It needs to be made `pub(crate)` so the top-level `Resume` variant can reference it in its `#[arg]` attribute. Alternatively, the function can be duplicated at the top level — but sharing is cleaner.

### Action semantics

Both commands use `DaemonClient::for_action()` (auto-starts daemon, max 1 restart), matching the existing `oj pipeline cancel/resume` semantics. This ensures the daemon is available when the user wants to cancel or resume.

## Verification Plan

1. **Unit tests** — CLI parsing tests in `main_tests.rs` (Phase 4)
2. **Build check** — `cargo build --all` and `cargo clippy --all -- -D warnings`
3. **Help text** — Visual inspection of `oj --help`, `oj cancel --help`, `oj resume --help`
4. **Manual smoke test** — Start a pipeline, use `oj cancel <id>` and `oj resume <id>` to verify they behave identically to `oj pipeline cancel <id>` and `oj pipeline resume <id>`
