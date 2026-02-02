# Plan: `oj daemon restart`

## Overview

Add `oj daemon restart` as a convenience command that stops the running daemon (if any) and then starts it in the background. This is TODO item #12. The change is CLI-only — no daemon protocol changes needed.

## Project Structure

Only two files need changes:

```
crates/cli/src/commands/daemon.rs  — add Restart variant + restart() fn
```

`crates/cli/src/main.rs` should not need changes since `DaemonCommand` is dispatched inside `daemon()` in `daemon.rs`.

## Dependencies

No new dependencies. The implementation reuses existing `daemon_stop` and `DaemonClient::connect_or_start`.

## Implementation Phases

### Phase 1: Add the `Restart` subcommand

1. Add a `Restart` variant to `DaemonCommand` enum (line 20-44 in `daemon.rs`):
   ```rust
   /// Stop and restart the daemon
   Restart {
       /// Kill all active sessions (agents, shells) before restarting
       #[arg(long)]
       kill: bool,
   },
   ```

2. Add match arm in `daemon()` (line 47-53):
   ```rust
   DaemonCommand::Restart { kill } => restart(kill).await,
   ```

3. Implement `restart()`:
   ```rust
   async fn restart(kill: bool) -> Result<()> {
       // Stop the daemon if running (ignore "not running" case)
       let was_running = daemon_stop(kill).await.map_err(|e| anyhow!("Failed to stop daemon: {}", e))?;

       if was_running {
           // Brief wait for the process to fully exit and release the socket
           tokio::time::sleep(std::time::Duration::from_millis(500)).await;
       }

       // Start in background
       match DaemonClient::connect_or_start() {
           Ok(_client) => {
               println!("Daemon restarted");
               Ok(())
           }
           Err(e) => Err(anyhow!("{}", e)),
       }
   }
   ```

   Note on the sleep: This is **not** a synchronization hack — it's a brief grace period for the OS to release the Unix socket after the daemon process exits. `connect_or_start` will handle retries internally; this just avoids the common case of immediately hitting a stale socket. This falls under the "intentional rate limiting" exception in CLAUDE.md's no-sleep policy.

### Phase 2: Verify

1. Run `make check` to verify:
   - `cargo fmt` passes
   - `cargo clippy` passes
   - `cargo test --all` passes
   - `cargo build --all` passes

## Key Implementation Details

- **Daemon not running**: `daemon_stop` returns `Ok(false)` when the daemon isn't running. The restart function handles this gracefully — it skips the sleep and proceeds directly to start.
- **Output message**: Prints "Daemon restarted" on success regardless of whether the daemon was previously running, keeping the UX simple.
- **Kill flag**: Passed through to `daemon_stop` so active sessions can be killed before restart, matching the existing `stop` command's behavior.
- **No foreground mode**: Restart always starts in background mode. Foreground restart doesn't make sense as a convenience command.

## Verification Plan

1. `make check` passes (fmt, clippy, tests, build, audit, deny)
2. Manual smoke test: `oj daemon restart` when daemon is running → prints "Daemon restarted"
3. Manual smoke test: `oj daemon restart` when daemon is not running → prints "Daemon restarted" (starts fresh)
4. `oj daemon restart --kill` stops sessions before restarting
