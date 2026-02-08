# Handler Cancellation — Future

## Problem

The CLI sets a 5-second timeout on IPC requests (`OJ_TIMEOUT_IPC_MS`). When
a daemon handler takes longer than 5 seconds (e.g., `WorkspacePrune` running
multiple `git worktree remove` calls, or `PeekSession` waiting on a hung
tmux), the CLI gives up and reports an error.

The daemon handler task, however, keeps running — it has no awareness that
the client disconnected. It continues blocking on the subprocess, eventually
completes, writes the response to a closed socket (silently fails), and
exits. If the user retries, a new handler task spawns while the old one is
still running.

This is not a resource leak (the orphaned task completes eventually), but
it is wasted work and can compound under repeated retries.

## Current State

No cancellation or client-disconnect detection exists. Each connection is
handled in a spawned tokio task with no awareness of client state.

Individual subprocess calls within handlers *do* have timeouts via
`run_with_timeout()` (e.g., `TMUX_TIMEOUT` 10s, `GIT_WORKTREE_TIMEOUT`
60s), but there is no overall handler-level timeout or cancellation. A
handler iterating over multiple subprocesses (e.g., pruning several
workspaces) can exceed the 5-second IPC timeout while each individual
subprocess stays within its own limit. `SessionPrune` is a notable gap —
it calls `tmux kill-session` without any timeout wrapper.

## Current Impact

Low. The scenario requires a handler that blocks for >5 seconds, which only
happens with subprocess-calling handlers (`PeekSession`, `WorkspacePrune`,
`AgentResume`, `SessionPrune`). These are infrequently used and the
orphaned tasks self-clean.

## When This Matters

This becomes a concern when:

- Deferred effects reduce event loop blocking, making the 5-second timeout
  the *primary* source of user-visible errors (currently masked by longer
  blocking)
- Handlers acquire resources (locks, file handles) that the orphaned task
  holds longer than expected
- Users script `oj` commands in loops, creating many concurrent retries

## Potential Fix

Thread a `CancellationToken` (from `tokio_util::sync`) through handler tasks.
Detect client disconnect via the socket half-close and trigger cancellation:

```rust
tokio::spawn(async move {
    let token = CancellationToken::new();
    let handler = handle_request(request, state, token.clone());
    let disconnect = detect_client_gone(&stream);

    tokio::select! {
        result = handler => { write_response(result).await; }
        _ = disconnect => { token.cancel(); }
    }
});
```

Subprocess-calling handlers check the token before each operation:

```rust
async fn handle_workspace_prune(state, token) -> Response {
    for workspace in workspaces {
        if token.is_cancelled() {
            return Response::Error { message: "cancelled".into() };
        }
        git_worktree_remove(&workspace).await;
    }
}
```

This requires:

1. Adding `tokio-util` as a dependency (or using a simple `AtomicBool` flag)
2. Threading the token through all subprocess-calling handlers
3. Detecting half-close on the Unix socket (read returning 0 bytes)
4. Deciding whether to propagate cancellation into subprocess kills
   (`Command::kill()`) or just abandon the await

The simpler alternative is handler-side timeouts: subprocess-calling IPC
handlers use `tokio::time::timeout()` with purpose-specific limits (see
`arch/03-concurrency.md`). This doesn't detect client disconnect but bounds
the maximum handler duration, which addresses the same symptom. The two
approaches compose well — timeouts prevent unbounded blocking, cancellation
prevents wasted work.
