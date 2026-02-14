# Daemon Architecture

The system splits into two processes: `oj` (CLI) and `ojd` (daemon).

## Purpose

The core purpose of oj is **background work dispatch** - running agents in isolated sessions while the user does other things. This requires a persistent process that:

1. Receives commands from CLI
2. Runs the event loop
3. Spawns and monitors agents
4. Drives jobs through steps
5. Persists state for crash recovery

## Process Architecture

```diagram
┌─────────────────────────────────────────────────────────────┐
│  oj (CLI)                                    crates/cli     │
│                                                             │
│  1. Find project root (walk up for .oj/)                    │
│  2. Connect to user's daemon (auto-start if needed)         │
│  3. Send request with project_path, receive response        │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          │  Unix socket: ~/.local/state/oj/daemon.sock
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  ojd (Daemon)                                crates/daemon  │
│                                                             │
│  Persistent process that owns the event loop:               │
│                                                             │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  Event Loop (main thread)                             │  │
│  │                                                       │  │
│  │  loop {                                               │  │
│  │      select! {                                        │  │
│  │          event = reader.recv() => process(event).     │  │
│  │          _ = shutdown.notified() => break             │  │
│  │          _ = sigterm/sigint => break                  │  │
│  │          _ = interval(1s) => check_timers()           │  │
│  │      }                                                │  │
│  │  }                                                    │  │
│  │                                                       │  │
│  │  Spawned tasks: listener, checkpoint, flush           │  │
│  └───────────────────────────────────────────────────────┘  │
│                                                             │
│  Owns:                                                      │
│  - State (jobs, agents, workspaces)                         │
│  - Adapters (agent, notify)                                 │
│  - Storage (WAL)                                            │
└─────────────────────────────────────────────────────────────┘
```

## Directory Layout

One daemon serves all projects for a user:

```example
~/.local/state/oj/
├── daemon.sock          # Unix socket
├── daemon.pid           # Lock file (contains PID)
├── daemon.version       # Version file (for mismatch detection)
├── daemon.log           # Daemon logs
├── snapshot.json        # State snapshot (zstd compressed)
├── wal/
│   └── events.wal       # Write-ahead log
├── logs/                # Per-job and per-agent logs
│   ├── <job-id>.log
│   └── agent/
│       ├── <agent-id>.log
│       └── <agent-id>/  # Agent session JSONL
├── agt-<agent-id>/      # Per-agent coop directories
│   ├── coop.sock        # Coop Unix socket
│   └── agent-config.json
└── workspaces/
    └── ws-<name>-<nonce>/  # Git worktrees for ephemeral workspaces
```

**Why user-level:**
- Simpler architecture (single daemon process)
- Cross-project visibility (one place to see all jobs)
- CLI passes `project_path` with each `RunCommand` request

## Why a Daemon?

Without a daemon, each CLI invocation would need to:
- Load state from WAL
- Execute one step
- Exit

This creates problems:
- **No continuous monitoring** - can't detect stuck agents
- **Race conditions** - multiple CLI invocations could conflict
- **No event loop** - shell completions can't trigger next steps
- **Inefficient** - constant WAL replay on every command

The daemon solves these by being a single owner of state and the event loop.

## IPC Protocol

**Transport:** Unix socket

**Format:** Length-prefixed JSON (4-byte big-endian length prefix + JSON payload)

Request categories:
- **Core**: Ping, Hello (version handshake), Status, Event, Query, Shutdown, RunCommand
- **Agent**: AgentSend, AgentResume, AgentKill, AgentAttach, AgentPrune
- **Job**: JobResume, JobResumeAll, JobCancel, JobSuspend, JobPrune
- **Workspace**: WorkspaceDrop, WorkspaceDropFailed, WorkspaceDropAll, WorkspacePrune
- **Worker**: WorkerStart, WorkerStop, WorkerRestart, WorkerResize, WorkerWake, WorkerPrune
- **Cron**: CronStart, CronStop, CronRestart, CronOnce, CronPrune
- **Queue**: QueuePush, QueueDrop, QueueRetry, QueueDrain, QueueFail, QueueDone, QueuePrune
- **Decision**: DecisionResolve

Queries read state without side effects: list/get for jobs, agents, workspaces, workers, crons, queues, decisions, and overview status.

Handlers fall into three categories by blocking behavior:
- **Event-emitting** (non-blocking): RunCommand, Event, QueuePush, WorkerStart/Stop, CronStart/Stop — write to WAL and return
- **State-reading** (blocks on `state.lock()`): All queries, JobCancel, JobResume, DecisionResolve
- **Subprocess-calling** (blocks on external process): AgentSend, AgentResume, WorkspacePrune — each has a purpose-specific timeout

## Event Loop

The daemon runs a continuous event loop:

```
┌─────────────────────────────────────────────────────────┐
│                     Event Sources                       │
│                                                         │
│  ┌─────────┐  ┌─────────────┐  ┌──────────┐             │
│  │   IPC   │  │  Internal   │  │  Timers  │             │
│  │ (socket)│  │   Queue     │  │          │             │
│  └────┬────┘  └──────┬──────┘  └────┬─────┘             │
│       │              │              │                   │
│       └──────────────┼──────────────┘                   │
│                      ▼                                  │
│              ┌───────────────┐                          │
│              │  Event Loop   │                          │
│              └───────┬───────┘                          │
│                      │                                  │
│                      ▼                                  │
│              ┌───────────────┐                          │
│              │    Runtime    │                          │
│              │  (engine)     │                          │
│              └───────┬───────┘                          │
│                      │                                  │
│                      ▼                                  │
│              ┌───────────────┐                          │
│              │   Effects     │───────┐                  │
│              └───────────────┘       │                  │
│                      │               │                  │
│      ┌───────────────┼───────────────┼──────┐           │
│      ▼               ▼               ▼      ▼           │
│  ┌───────┐     ┌─────────┐    ┌──────┐ ┌───────┐        │
│  │ agent │     │ notify  │    │ WAL  │ │ Queue │        │
│  │Adapter│     │ Adapter │    │      │ │       │        │
│  └───────┘     └─────────┘    └──────┘ └───────┘        │
│                                              │          │
│                                              │          │
│                      Internal events ◄───────┘          │
│                      (ShellExited, etc.)                │
└─────────────────────────────────────────────────────────┘
```

Effects that produce events (like `Effect::Shell`) feed results back into the internal queue, creating the progression chain.

## Lifecycle

### Startup

```
1. Write startup marker to log ("--- ojd: starting (pid: <pid>)")
2. Acquire lock file (prevent multiple daemons)
3. Create directories, write version file
4. Load snapshot, then replay WAL
5. Set up adapters and runtime
6. Bind socket
7. Print READY, enter event loop
8. Reconcile with reality (background task, after READY)
```

**Startup Error Reporting:**

When the CLI starts the daemon and it fails, errors are reported via the log:

1. Daemon writes `--- ojd: starting (pid: <pid>)` marker before anything else
2. CLI waits for socket to appear (with timeout)
3. If timeout, CLI reads log from last marker, extracts ERROR lines
4. Error message shown to user instead of generic "timeout"

This ensures runbook parse errors, permission issues, etc. are visible to the user.

### Shutdown

By default, `oj daemon stop` preserves agent processes. Agents continue running
independently — on next startup, the reconciliation flow reconnects to survivors
and resumes job progression. This is critical for long-running agents that
may take hours; a daemon restart (e.g., for version upgrade) should not kill work
in progress.

```
1. Break event loop
2. Flush buffered WAL events to disk
3. Save final snapshot (fast subsequent startup)
4. Remove socket file (stops new connections)
5. Remove PID and version files
6. Release lock file
7. Exit (sessions left alive)
```

Use `oj daemon stop --kill` to terminate all sessions before stopping. This kills
every session tracked in the daemon's state (scoped to this daemon instance, safe
for parallel runs). Kills happen in the IPC request handler before the response is
sent, so they complete before the CLI's exit timer starts.

### Recovery

On restart after crash (or normal restart with surviving sessions):

```
1. Replay WAL to reconstruct state
2. Reconcile:
   - Prune orphaned sessions from terminal/missing jobs
   - Resume running workers (re-emit WorkerStarted)
   - Resume running crons (re-emit CronStarted)
   - For crew and jobs:
     check agent processes
3. Reconnect watchers or trigger on_dead actions
```

The WAL records intent; reconciliation bridges the gap with reality. This is why
session preservation is the default — the restart+reconcile flow is designed to
pick up exactly where the daemon left off.

## CLI Connection Protocol

CLI finds the project root by walking up from cwd looking for `.oj/` directory:

```example
<project>/
└── .oj/
    ├── config.toml          # Project config (optional)
    └── runbooks/            # Runbook files
        ├── build.hcl
        ├── bugfix.hcl
        └── ...
```

Then connects to the daemon (auto-starting if needed):

```mermaid
flowchart LR
    A[Check socket] --> B{Exists?}
    B -->|No| C[Start daemon]
    B -->|Yes| D{Can connect?}
    D -->|No| E[Cleanup stale]
    E --> C
    D -->|Yes| F{Version match?}
    F -->|No| G[Stop daemon]
    G --> C
    F -->|Yes| H[Send request]
    C --> I[Poll for ready]
    I -->|Timeout| J[Extract errors from log]
    I -->|Ready| D
```

1. **Check socket file**
   - If socket doesn't exist → start daemon

2. **Attempt connection**
   - If socket exists but connect fails → stale socket from crashed daemon

3. **Cleanup stale files**
   1. Check if PID file exists
   2. If PID exists, verify process is dead via `kill(pid, 0)`
   3. If dead (or no PID file), remove socket and PID files
   4. Proceed with fresh daemon start

4. **Version check**
   1. Compare CLI version against `daemon.version` file
   2. If mismatch, print warning
   3. Stop old daemon via SIGTERM (graceful, then force-kill if needed)
   4. Start fresh daemon with current version

5. **Poll for daemon ready**
   - Poll socket with 50ms interval (`OJ_CONNECT_POLL_MS`)
   - Timeout after 5s (`OJ_TIMEOUT_CONNECT_MS`)
   - Non-blocking `try_wait()` detects early daemon exit

6. **Error reporting on timeout**
   1. Read daemon log file
   2. Find last startup marker (`--- ojd: starting (pid: <pid>)`)
   3. Extract ERROR lines after marker
   4. Display to user instead of generic "connection timeout"

## Daemon Startup Sequence

```mermaid
flowchart LR
    A[Log marker] --> B[Acquire lock]
    B -->|Held| C[Exit]
    B -->|OK| D[Create dirs]
    D --> E[Load snapshot]
    E --> F[Replay WAL]
    F --> G[Bind socket]
    G --> H[Print READY]
    H --> I[Event loop]
    I --> J[Reconcile agents]
```

1. **Write log marker**
   - Write `--- ojd: starting (pid: <pid>)` before anything else
   - CLI uses this to find relevant errors on startup failure

2. **Acquire lock**
   - Open PID file without truncating (preserves running daemon's PID)
   - Non-blocking `try_lock_exclusive()` on PID file
   - Fails immediately if another daemon holds lock (no waiting)
   - Only after lock is acquired: truncate and write current PID
   - OS releases lock automatically when process dies
   - On `LockFailed`, skip cleanup—existing files belong to the running daemon

3. **Create directories**
   - `create_dir_all()` for: socket parent, WAL parent, workspaces
   - Idempotent—safe if directories already exist
   - Write `daemon.version` file (CLI uses this for version mismatch detection)

4. **Load snapshot**
   - Parse JSON snapshot file
   - On corruption:
     1. Move corrupt file to `.bak`
     2. Log warning with error details
     3. Continue with empty state

5. **Replay WAL**
   - Parse each JSON line after snapshot sequence number
   - On corruption:
     1. Collect valid entries before corruption point
     2. Rename corrupt WAL to `.bak`
     3. Create clean WAL with valid entries only
     4. Log warning with entry count
   - Backup rotation: `.bak` → `.bak.2` → `.bak.3` (max 3 kept)

6. **Set up adapters and runtime**
   - Create traced adapters (coop agent) and notify adapter
   - Create internal event channel for runtime-produced events
   - Spawn runtime event forwarder (internal channel → EventBus)

7. **Bind socket**
   - Remove stale socket file (safe—lock already acquired)
   - Bind to socket path
   - On failure, cleanup partial state (socket, PID, version files)
   - Cleanup only runs for post-lock failures; lock contention skips cleanup entirely

8. **Print READY**
   - Signal that daemon is accepting connections
   - CLI polls for socket availability after this

9. **Enter event loop**
   - Spawn async tasks: listener, checkpoint, flush
   - No blocking—all work in separate tasks

10. **Reconcile state** (background)
    - Spawned AFTER ready—doesn't block CLI
    - **Sessions:** Prune orphaned sessions whose jobs are terminal or missing
    - **Workers:** Re-emit `WorkerStarted` for each running worker (resumes queue polling)
    - **Crons:** Re-emit `CronStarted` for each running cron (resumes scheduling)
    - **Crew:** Same 3-case check as jobs (below)
    - **Jobs:** For each non-terminal job, check agent state:

    | Condition | Action |
    |-----------|--------|
    | Session alive, running | Reconnect WebSocket bridge |
    | Session alive, agent dead | Emit `AgentExited` |
    | Session dead | Emit `AgentGone` |

    - Jobs in `Waiting` state skipped (already escalated to human)

### Atomic Writes and Durability

Snapshots use the full durability sequence for crash safety:

1. Serialize state + compress with zstd (background thread)
2. Write to `.tmp` file
3. `fsync` the temp file (data durable)
4. Atomic rename to final path
5. `fsync` parent directory (rename durable)
6. **Only then** truncate WAL

The directory fsync is critical: without it, a power failure after rename but
before the directory is synced could revert to the old snapshot while the WAL
has already been truncated, losing events.

WAL truncation also uses temp-then-rename. Regular WAL writes are append-only
with buffered group commit (~10ms window) followed by `sync_all()`.

### Background Checkpoints

Checkpoint I/O runs off the main thread to minimize latency:

```diagram
Main Thread                 Background Thread
clone state (~10ms)    →    serialize + compress + I/O (~200ms)
continue processing         └→ completion signal
                       ←
truncate WAL
```

At 1-2k jobs, this keeps main thread blocking under 10ms while the full
checkpoint (including compression and fsyncs) takes ~200ms in the background.

## Daemon Management

```bash
oj daemon start                 # Start daemon (background)
oj daemon start --foreground    # Start in foreground (debugging)
oj daemon status                # Check status
oj daemon stop                  # Graceful shutdown (sessions preserved)
oj daemon stop --kill           # Stop and terminate all sessions
oj daemon restart               # Stop and restart
oj daemon logs [-f] [-n 200]    # View logs (default 200 lines)
oj daemon orphans               # List orphaned jobs from startup
```

### Auto-Start

The daemon auto-starts on first command if not already running:

```
connect_or_start():
    if can connect to socket:
        return connection
    else:
        start daemon in background
        retry connect with 5s timeout
        return connection
```

This provides seamless UX - users don't need to think about daemon lifecycle for normal usage. Explicit `oj daemon start` is only needed for debugging or custom configurations.

## Environment Variables

### CLI

| Variable | Default | Description |
|----------|---------|-------------|
| `OJ_STATE_DIR` | `~/.local/state/oj` | Base directory for all daemon state (socket, PID, WAL, workspaces, logs). Falls back to `$XDG_STATE_HOME/oj`. Useful for test isolation. |
| `OJ_PROJECT` | Auto-detected | Project project for resource isolation. Auto-detected from `.oj/config.toml [project].name` or directory basename. Propagated to agents and shell steps so nested `oj` calls inherit the parent project's project. |
| `OJ_DAEMON_BINARY` | Auto-detected | Path to the `ojd` binary. Auto-detected from the current executable's location. |
| `OJ_TIMEOUT_IPC_MS` | `5000` | Timeout for IPC requests between CLI and daemon. |
| `OJ_TIMEOUT_CONNECT_MS` | `5000` | Timeout for waiting for daemon to start when auto-starting. |
| `OJ_TIMEOUT_EXIT_MS` | `2000` | Timeout for graceful process exit before force-kill. |
| `OJ_CONNECT_POLL_MS` | `50` | Polling interval for connection retries to daemon socket. |
| `OJ_RUN_WAIT_MS` | `10000` | Initial wait after `oj run` spawns a job before returning. |
| `OJ_WAIT_POLL_MS` | `1000` | Polling interval for `oj job wait` and `oj agent wait`. |

### Daemon / Adapters

| Variable | Default | Description |
|----------|---------|-------------|
| `OJ_IDLE_GRACE_MS` | `60000` | Grace period before confirming idle. After an `AgentIdle` event, the engine waits this long, then re-checks log growth and agent state before triggering `on_idle`. |
| `OJ_PROMPT_POLL_MS` | `3000` | Timeout for detecting and handling Claude Code prompts (permissions bypass, workspace trust). |
| `OJ_SESSION_POLL_MS` | `1000` | Polling interval while waiting for an agent's session log to appear after spawn. |
| `OJ_WATCHER_POLL_MS` | `5000` | Fallback polling interval for agent watcher when file-based monitoring isn't available. |
| `OJ_TIMER_CHECK_MS` | `1000` | Interval for the main loop's timer check branch (how often fired timers are collected). |

## See Also

- [Overview](00-overview.md) - System architecture
- [Effects](02-effects.md) - Effect types
- [Storage](04-storage.md) - WAL persistence
