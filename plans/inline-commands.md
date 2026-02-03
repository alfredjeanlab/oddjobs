# Inline Commands

## Overview

Teach `oj run` to execute shell commands inline (in the current process, stdout to terminal) instead of dispatching everything through the daemon. When a command's `run` directive is a plain shell string (`RunDirective::Shell`), execute it locally via `std::process::Command` with inherited stdio. Pipeline and agent directives continue to dispatch through the daemon as before.

This eliminates the overhead of daemon round-trips, WAL persistence, and pipeline polling for simple one-shot scripts, and gives users direct terminal output (colors, interactivity, ctrl+c).

## Project Structure

Files to create or modify:

```
crates/cli/src/commands/run.rs   # Main changes: local execution path for Shell directives
crates/runbook/src/lib.rs        # (no changes expected)
```

No new files or crates are needed. The change is confined to the CLI's run command handler.

## Dependencies

No new external dependencies. Uses `std::process::Command` from the standard library.

## Implementation Phases

### Phase 1: Local Shell Execution in CLI

**Goal:** When `oj run <command>` resolves to a `RunDirective::Shell`, execute the command locally instead of dispatching to the daemon.

**Changes to `crates/cli/src/commands/run.rs`:**

1. After prevalidation and argument parsing (line ~141), inspect `cmd_def.run`:
   - If `RunDirective::Shell(cmd)` → execute locally (new code path)
   - If `RunDirective::Pipeline { .. }` or `RunDirective::Agent { .. }` → dispatch to daemon (existing code path)

2. The local execution path:

```rust
RunDirective::Shell(cmd) => {
    // Build interpolation variables
    let parsed_args = cmd_def.parse_args(&positional, &named);
    let mut vars: HashMap<String, String> = parsed_args
        .iter()
        .map(|(k, v)| (format!("args.{}", k), v.clone()))
        .collect();
    vars.insert("invoke.dir".to_string(), invoke_dir.display().to_string());

    // Interpolate the shell command template
    let interpolated = oj_runbook::interpolate_shell(&cmd, &vars);

    // Execute locally with inherited stdio
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&interpolated)
        .current_dir(project_root)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .env("OJ_NAMESPACE", namespace)
        .status()?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        std::process::exit(code);
    }

    Ok(())
}
```

3. Move the daemon dispatch into the Pipeline/Agent match arms. The function signature stays the same — `client` is still passed but only used for non-Shell directives.

**Key decisions:**
- **No `local = true` marker.** Infer from directive type: `Shell` → local, `Pipeline`/`Agent` → daemon. This is the simplest approach and matches user expectations (shell commands are fast local operations; pipelines need orchestration).
- **`cwd` is `project_root`**, matching the engine's current behavior for shell commands (see `command.rs:98`).
- **Propagate `OJ_NAMESPACE`** as an environment variable so nested `oj` calls work.
- **Exit code forwarding.** If the shell command fails, exit with its code rather than returning an error message.

### Phase 2: Variable Parity with Engine

**Goal:** Ensure local execution has the same variable interpolation as the daemon engine path, minus pipeline-specific variables that don't apply.

The engine (`crates/engine/src/runtime/handlers/command.rs:141-152`) provides these variables for shell commands:
- `args.*` — from parsed command arguments
- `pipeline_id` — UUID of the pipeline record
- `name` — pipeline name (defaults to pipeline_id)
- `workspace` — execution path (same as project_root for shell commands)
- `invoke.dir` — directory where `oj run` was invoked

For local execution:
- `args.*` — **yes**, same as engine
- `invoke.dir` — **yes**, same as engine
- `workspace` — **yes**, set to `project_root` (matches engine behavior for shell commands)
- `pipeline_id` — **omit** (no pipeline exists); leave `${pipeline_id}` uninterpolated
- `name` — **omit** (no pipeline exists); leave `${name}` uninterpolated

The `interpolate_shell` function already leaves unknown variables as-is, so omitting `pipeline_id` and `name` is safe.

### Phase 3: Support `--daemon` Escape Hatch

**Goal:** Allow users to force daemon dispatch even for shell commands, for cases where they want WAL logging, pipeline tracking, or daemon-side execution.

Add a `--daemon` flag to `RunArgs`:

```rust
/// Force execution through the daemon (even for shell commands)
#[arg(long)]
pub daemon: bool,
```

When `--daemon` is set, skip the local execution check and always dispatch to the daemon. This is a simple boolean check before the `match cmd_def.run` block.

### Phase 4: Tests

**Goal:** Verify local execution works correctly and daemon dispatch is preserved.

**Unit tests in `crates/cli/src/commands/run.rs` (or a `run_tests.rs` companion):**

1. **Shell command executes locally** — mock a runbook with `RunDirective::Shell("echo hello")`, verify `handle` spawns a local process (not a daemon call). This may require extracting the execution logic into a testable function that returns the interpolated command + execution mode rather than directly spawning.

2. **Pipeline command dispatches to daemon** — verify `RunDirective::Pipeline` still calls `client.run_command()`.

3. **Variable interpolation** — verify `args.*`, `invoke.dir`, and `workspace` are correctly interpolated in the local path.

4. **`--daemon` flag forces daemon dispatch** — verify shell commands go through daemon when flag is set.

5. **Exit code forwarding** — verify non-zero exit codes from shell commands propagate.

**Integration test (if feasible):**

Create a test runbook with a shell command that writes to a file, run `oj run <command>`, and verify the file exists. This validates the full path without needing a daemon.

## Key Implementation Details

### Execution Flow (After Change)

```
oj run <command> [args]
  │
  ├─ Load runbook, validate command, parse args (unchanged)
  │
  ├─ match cmd_def.run:
  │   ├─ Shell(cmd) && !args.daemon
  │   │   ├─ Build vars: args.*, invoke.dir, workspace
  │   │   ├─ interpolate_shell(cmd, vars)
  │   │   └─ std::process::Command::new("sh").arg("-c").arg(interpolated)
  │   │       → inherited stdio, exit code forwarded
  │   │
  │   ├─ Pipeline { .. } | Agent { .. }
  │   │   └─ client.run_command() → daemon (existing path)
  │   │
  │   └─ Shell(cmd) && args.daemon
  │       └─ client.run_command() → daemon (existing path)
  │
  └─ done
```

### Why Not a Runbook Marker

A `local = true` marker in the runbook was considered but rejected because:
- It adds complexity to the runbook schema for no gain
- The directive type already encodes intent: shell commands are inherently local; pipelines need orchestration
- Users can override with `--daemon` if they need daemon behavior for a shell command
- Adding it later is non-breaking if the need arises

### Signal Handling

`std::process::Command` with inherited stdio means ctrl+c propagates naturally to the child process (same process group). No special signal handling is needed.

### No Daemon Connection for Shell-Only Runs

Currently `handle` takes a `&DaemonClient`. For shell-only execution, the daemon connection is unnecessary. The implementation should avoid connecting to the daemon when it's not needed. This can be done by:
- Restructuring `handle` to perform prevalidation and directive inspection before requiring a client
- Or accepting that the client connection is cheap and already established by the time `handle` is called

The pragmatic choice is to leave the function signature unchanged and simply not use the client for shell commands. Optimizing away the daemon connection can be a follow-up.

## Verification Plan

1. **`make check`** — `cargo fmt`, `clippy`, `cargo test --all`, `cargo build --all` all pass
2. **Manual test** — Create a test command in `.oj/runbooks/test.hcl`:
   ```hcl
   command "hello" {
     args = "<name>"
     run  = "echo Hello, ${args.name}! cwd=$(pwd)"
   }
   ```
   Run `oj run hello world` and verify:
   - Output appears directly in terminal (not "Waiting for pipeline to start...")
   - `cwd` shows the project root
   - Exit code 0
3. **Manual test (failure)** — `run = "exit 42"`, verify `oj run` exits with code 42
4. **Manual test (daemon path)** — `oj run hello world --daemon`, verify it goes through the daemon (shows "Waiting for pipeline..." message)
5. **Manual test (pipeline)** — Verify existing pipeline commands still work through the daemon
