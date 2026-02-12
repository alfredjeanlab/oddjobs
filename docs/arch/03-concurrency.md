# Concurrency

How threads, tasks, locks, and I/O interact in the daemon.

## Thread Model

The daemon runs on a tokio multi-threaded runtime (`#[tokio::main]` with
`features = ["full"]`). No explicit runtime builder overrides; worker thread
count defaults to the number of CPU cores.

```diagram
OS Threads
──────────────────────────────────────────────────────────
  tokio workers       N (= CPU cores, default)
  tokio blocking      transient (checkpoint wait, desktop notify)
──────────────────────────────────────────────────────────
  Typical total       N   (+ transient blocking threads)
```

## Task Topology

All concurrency is expressed as tokio tasks on the shared worker pool. There
are no long-lived OS threads besides the notify-crate watcher thread.

```diagram
daemon process
│
├─ main task ─────────── event loop (select!)
│
├─ listener task ─────── accept loop
│   ├─ connection task ─ handle_connection (one per IPC request)
│   ├─ connection task
│   └─ ...
│
├─ flush task ────────── WAL group commit (10ms interval)
├─ checkpoint task ───── snapshot + WAL truncate (60s interval)
├─ event forwarder ───── runtime mpsc → EventBus bridge
│
├─ agent watcher task ── per-agent file watcher + liveness poll
├─ agent watcher task
├─ ...
│
├─ deferred effect task ─ per-effect background I/O (workspace, agent, session)
├─ shell task ────────── fire-and-forget bash execution (per Shell effect)
├─ queue poll task ───── fire-and-forget queue list command
├─ agent log writer ──── mpsc → append-only log files
│
└─ reconciliation task ─ one-shot startup recovery (then exits)
```

## Event Loop

The daemon's core loop in `daemon/src/main.rs` multiplexes five sources with
`tokio::select!`:

```diagram
┌──────────────────────────────────────────────────────────────┐
│                      tokio::select!                          │
│                                                              │
│  ┌────────────────┐                                          │
│  │ event_reader   │─► process_event(event).await             │
│  │ (WAL)          │   ├─ state.lock() + apply_event()        │
│  └────────────────┘   ├─ runtime.handle_event().await        │
│                       │   └─ executor.execute_all().await     │
│  ┌────────────────┐   └─ event_bus.send() per result event   │
│  │ shutdown_notify│─► break                                  │
│  └────────────────┘                                          │
│  ┌────────────────┐                                          │
│  │ SIGTERM/SIGINT │─► break                                  │
│  └────────────────┘                                          │
│  ┌────────────────┐                                          │
│  │ timer interval │─► scheduler.fired_timers() → WAL         │
│  │ (1s default)   │                                          │
│  └────────────────┘                                          │
└──────────────────────────────────────────────────────────────┘
```

The loop processes **one event at a time**. Each `process_event()` iteration
completes in <10ms because all heavy I/O effects are deferred to background
tasks (see Effect Execution Model below). Timer resolution stays at the
configured interval (default 1s).

## Effect Execution Model

Effects are executed by `executor.execute_all()` in a **sequential for-loop**.
Each effect in a batch is awaited before the next begins. Effects are split
into **immediate** (executed inline, <10ms) and **deferred** (spawned as
background `tokio::spawn` tasks):

```diagram
┌──────────────────────────────────────────────────────────────────────────┐
│                          execute_all()                                    │
│                                                                          │
│  for effect in effects {                                                 │
│      self.execute(effect).await   ◄── sequential, one at a time          │
│  }                                                                       │
│                                                                          │
│  ┌─────────────────────────────┐  ┌────────────────────────────────────┐ │
│  │  Immediate (<10ms)          │  │  Deferred (tokio::spawn)           │ │
│  │                             │  │                                    │ │
│  │  Emit          ~µs          │  │  CreateWorkspace  → WorkspaceReady │ │
│  │  SetTimer      ~µs          │  │  DeleteWorkspace  → WorkspaceDeleted│ │
│  │  CancelTimer   ~µs          │  │  SpawnAgent       → AgentSpawned   │ │
│  │  Notify        ~1ms  [1]    │  │  SendToAgent      (fire-and-forget)│ │
│  │                             │  │  KillAgent        (fire-and-forget)│ │
│  │  [1] fire-and-forget via    │  │                                    │ │
│  │      spawn_blocking         │  │                                    │ │
│  │                             │  │  Shell            → ShellExited    │ │
│  └─────────────────────────────┘  │  PollQueue        → WorkerPolled│
│                                   │  TakeQueueItem    → WorkerTook│
│                                   │                                    │ │
│                                   │  Result events emitted via         │ │
│                                   │  mpsc → EventBus on completion     │ │
│                                   └────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────────┘
```

Deferred effects return immediately after spawning the background task. The
event loop never blocks on I/O-heavy operations like git worktree creation,
agent spawning, or agent communication.

Sequential dependencies are handled via event-driven chaining. A `CommandRun`
event that creates a workspace and spawns an agent progresses through multiple
event iterations, each completing in <10ms:

```diagram
process_event(CommandRun)
  └─ handler emits JobCreated + dispatches CreateWorkspace (deferred)
     └─ returns immediately

process_event(WorkspaceReady)        ◄── emitted by background task
  └─ handler calls start_step() → dispatches SpawnAgent (deferred)
     └─ returns immediately

process_event(AgentSpawned)          ◄── emitted by background task
  └─ handler sets up monitoring, timers
     └─ returns immediately
```

The total wall-clock time for workspace creation + agent spawn is the same,
but the event loop remains responsive throughout — timers fire, other events
process, and IPC queries respond without delay.

## Listener and IPC

The listener runs in a separate tokio task, accepting connections independently
of the event loop. Each connection spawns its own handler task.

```diagram
listener task (always running)
│
└─ loop { socket.accept() }
     └─ tokio::spawn(handle_connection)
          ├─ read_request()     5s timeout
          ├─ handle_request()   dispatches to handler
          └─ write_response()   5s timeout
```

Handlers fall into three categories by blocking behavior:

**Event-emitting** (non-blocking, <1ms):
`RunCommand`, `Event`, `QueuePush`, `WorkerStart/Stop`, `CronStart/Stop`
— write to WAL and return. Never contend with the engine.

**State-reading** (blocks on `state.lock()`):
All `Query::*` variants, `JobCancel`, `JobResume`, `AgentSend`,
`DecisionResolve` — acquire the shared `Mutex<MaterializedState>`. Blocked
whenever `process_event()` holds the lock.

**Subprocess-calling** (blocks on external process, with timeouts):
`AgentSend`, `AgentResume`, `WorkspacePrune`
— run agent or git subprocesses. Each has a purpose-specific timeout:

| Handler | Timeout | Operation |
|---------|---------|-----------|
| `AgentResume` | 5s per agent | agent process kills |
| `WorkspacePrune` | 30s per workspace | git worktree operations |

## Synchronization Primitives

All mutexes are `parking_lot::Mutex` (synchronous, non-async). No
`tokio::sync::Mutex` or `RwLock` is used.

```diagram
Shared State                        Protected By              Held Across .await?
─────────────────────────────────── ───────────────────────── ───────────────────
MaterializedState                   Arc<Mutex<..>>            No
Wal                                 Arc<Mutex<..>>            No
Scheduler                           Arc<Mutex<..>>            No
Runtime.agent_owners                Mutex<HashMap<..>>        No
Runtime.runbook_cache               Mutex<HashMap<..>>        No
Runtime.worker_states               Mutex<HashMap<..>>        No
Runtime.cron_states                 Mutex<HashMap<..>>        No
LocalAdapter.agents             Arc<Mutex<HashMap<..>>>   No
Vec<Breadcrumb> (orphans)           Arc<Mutex<Vec<..>>>       No
```

Locks are always acquired in scoped blocks and released before any `.await`.
No nested locking occurs.

**Channels:**

```diagram
Channel                             Type                  Capacity   Direction
─────────────────────────────────── ───────────────────── ────────── ──────────
Runtime → EventBus                  tokio::sync::mpsc     100        events
EventBus → EventReader              tokio::sync::mpsc     1          wake signal
Agent log entries                   tokio::sync::mpsc     256        log job
Watcher shutdown                    tokio::sync::oneshot  —          per-agent
Daemon shutdown                     tokio::sync::Notify   —          broadcast
CLI output streaming                tokio::sync::mpsc     16         CLI display
```

**Atomics:** `AtomicBool` for CLI restart guard, `AtomicU64` for sequential
ID generation.

## Blocking I/O on Worker Threads

`spawn_blocking` is used in two places: checkpoint completion wait
(`daemon/src/main.rs`) and desktop notifications (`adapters/src/notify/desktop.rs`).
All other blocking file I/O runs directly on tokio worker threads:

| Location | Operation |
|----------|-----------|
| `agent/coop/spawn.rs` `prepare_workspace()` | `fs::create_dir_all`, `fs::write` |
| `listener/query.rs` (multiple handlers) | `fs::read_to_string` for logs |
| `storage/snapshot.rs` `save()` | `File::create`, `serde_json::to_writer`, `sync_all` |
| `storage/wal.rs` `flush()` | `write_all`, `sync_all` |
| `engine/agent_logger.rs` writer task | `OpenOptions::open`, `writeln!` |

## Agent Watcher Model

Each running agent gets a tokio task that monitors agent state via coop's
WebSocket event bridge. The `LocalAdapter` subscribes to the agent's Unix
socket at `/ws?subscribe=state` and translates coop state events into engine
events (`AgentWorking`, `AgentIdle`, `AgentPrompt`, `AgentFailed`, `AgentGone`).

```diagram
agent watcher task (per agent)
│
├─ connect to coop WebSocket (Unix socket)
│
└─ event loop
     ├─ ws_message.recv()            translate coop state → emit Event
     ├─ liveness poll (periodic)     check agent process health
     └─ shutdown_rx                  oneshot from agent kill
```

Coop monitors the agent process directly and reports exit events with the
exit code over the WebSocket connection.

## Known Blocking Paths

These are the paths where the event loop or IPC handlers are blocked for
extended periods in the current implementation:

### Event loop — no inline blocking effects

All I/O effects are now deferred to background tasks. The event loop processes
only microsecond-scale inline effects (`Emit`, `SetTimer`, `CancelTimer`) and
the ~1ms `Notify` effect. No effect blocks the event loop for more than a few
milliseconds.

### Queries blocked by state lock

The state lock is **not** held across long `.await` points — it is acquired
and released in brief scoped blocks (during `apply_event()` in
`process_event()`). Query handlers can interleave between these brief
acquisitions. In practice, lock contention is low.

### Subprocess-calling IPC handlers

IPC handlers that call subprocesses (`WorkspacePrune`,
`AgentResume`) block their connection task on external process I/O. Each has
a purpose-specific timeout to bound the blocking duration (see Listener and
IPC section above).

## See Also

- [Daemon](01-daemon.md) - Process architecture, lifecycle, IPC protocol
- [Effects](02-effects.md) - Effect types and execution
- [Storage](04-storage.md) - WAL and snapshot persistence
- [Agents](05-agents.md) - Agent adapter and coop integration
