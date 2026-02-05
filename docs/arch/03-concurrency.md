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
  tokio blocking      0 active (spawn_blocking never used)
  notify crate        1 shared (kqueue/inotify internal)
  desktop notify      1 per Notify effect (unbounded, fire-and-forget)
──────────────────────────────────────────────────────────
  Typical total       N + 1   (+ transient notification threads)
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

The loop processes **one event at a time**. While `process_event()` is
awaiting, no other branch runs — timers, signals, and subsequent events wait
until the current event completes.

## Effect Execution Model

Effects are executed by `executor.execute_all()` in a **sequential for-loop**.
Each effect in a batch is awaited before the next begins. Effects fall into
two categories:

```diagram
┌──────────────────────────────────────────────────────────────────────────┐
│                          execute_all()                                    │
│                                                                          │
│  for effect in effects {                                                 │
│      self.execute(effect).await   ◄── sequential, one at a time          │
│  }                                                                       │
│                                                                          │
│  ┌─────────────────────────────┐  ┌────────────────────────────────────┐ │
│  │  Inline (awaited)           │  │  Background (spawned)              │ │
│  │                             │  │                                    │ │
│  │  Emit          ~µs          │  │  Shell        tokio::spawn         │ │
│  │  SetTimer      ~µs          │  │  PollQueue    tokio::spawn         │ │
│  │  CancelTimer   ~µs          │  │                                    │ │
│  │  Notify        ~1ms  [1]    │  │  Result events emitted via         │ │
│  │  SendToSession ~100ms       │  │  mpsc → EventBus on completion     │ │
│  │  KillSession   ~100ms       │  │                                    │ │
│  │  SendToAgent   ~200ms-2.2s  │  └────────────────────────────────────┘ │
│  │  KillAgent     ~100-300ms   │                                        │
│  │  SpawnAgent    ~1-9s  [2]   │                                        │
│  │  CreateWorkspace ~1-30s [3] │                                        │
│  │  DeleteWorkspace ~1-30s [3] │                                        │
│  │  TakeQueueItem  variable    │                                        │
│  │                             │                                        │
│  │  [1] fire-and-forget via    │                                        │
│  │      std::thread::spawn     │                                        │
│  │  [2] prompt polling loops   │                                        │
│  │  [3] git subprocess         │                                        │
│  └─────────────────────────────┘                                        │
└──────────────────────────────────────────────────────────────────────────┘
```

Inline effects block the event loop for their full duration. A `CommandRun`
event that creates a workspace and spawns an agent executes this chain
sequentially:

```diagram
process_event(CommandRun)
  └─ handle_command()
       ├─ load runbook from disk               ~100ms   (blocking file I/O)
       ├─ evaluate workspace.ref expression     ~50-500ms (bash subprocess)
       ├─ CreateWorkspace effect                ~1-30s   (git worktree add)
       ├─ SpawnAgent effect                     ~1-9s    (tmux + prompt polls)
       ├─ SetTimer effect                       ~µs
       └─ Notify effect                         ~1ms
                                        Total:  ~3-40s
```

During this window, the event loop cannot process timers, signals, or other
events.

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
All `Query::*` variants, `PipelineCancel`, `PipelineResume`, `SessionSend`,
`DecisionResolve` — acquire the shared `Mutex<MaterializedState>`. Blocked
whenever `process_event()` holds the lock.

**Subprocess-calling** (blocks on external process):
`SessionKill`, `PeekSession`, `AgentSend`, `AgentResume`, `WorkspacePrune`
— run tmux or git subprocesses. Can block for seconds even when the engine
is idle.

## Synchronization Primitives

All mutexes are `parking_lot::Mutex` (synchronous, non-async). No
`tokio::sync::Mutex`, `RwLock`, or `spawn_blocking` is used.

```diagram
Shared State                        Protected By              Held Across .await?
─────────────────────────────────── ───────────────────────── ───────────────────
MaterializedState                   Arc<Mutex<..>>            No
Wal                                 Arc<Mutex<..>>            No
Scheduler                           Arc<Mutex<..>>            No
Runtime.agent_pipelines             Mutex<HashMap<..>>        No
Runtime.agent_runs                  Mutex<HashMap<..>>        No
Runtime.runbook_cache               Mutex<HashMap<..>>        No
Runtime.worker_states               Mutex<HashMap<..>>        No
Runtime.cron_states                 Mutex<HashMap<..>>        No
ClaudeAgentAdapter.agents           Arc<Mutex<HashMap<..>>>   No
Vec<Breadcrumb> (orphans)           Arc<Mutex<Vec<..>>>       No
```

Locks are always acquired in scoped blocks and released before any `.await`.
No nested locking occurs. The parking_lot reentrancy test in
`lifecycle_tests.rs` validates this discipline.

**Channels:**

```diagram
Channel                             Type                  Capacity   Direction
─────────────────────────────────── ───────────────────── ────────── ──────────
Runtime → EventBus                  tokio::sync::mpsc     100        events
EventBus → EventReader              tokio::sync::mpsc     1          wake signal
Agent log entries                   tokio::sync::mpsc     256        log pipeline
Watcher shutdown                    tokio::sync::oneshot  —          per-agent
Daemon shutdown                     tokio::sync::Notify   —          broadcast
CLI output streaming                tokio::sync::mpsc     16         CLI display
```

**Atomics:** `AtomicBool` for CLI restart guard, `AtomicU64` for sequential
ID generation.

## Blocking I/O on Worker Threads

No `spawn_blocking` is used anywhere. All blocking file I/O runs directly on
tokio worker threads:

| Location | Operation |
|----------|-----------|
| `agent/claude.rs` `prepare_workspace()` | `fs::create_dir_all`, `fs::copy` |
| `agent/claude.rs` `session_log_size()` | `tokio::fs::metadata` (async) |
| `agent/watcher.rs` `parse_session_log()` | `File::open`, `BufReader::lines` |
| `listener/query.rs` (multiple handlers) | `fs::read_to_string` for logs |
| `storage/snapshot.rs` `save()` | `File::create`, `serde_json::to_writer`, `sync_all` |
| `storage/wal.rs` `flush()` | `write_all`, `sync_all` |
| `engine/agent_logger.rs` writer task | `OpenOptions::open`, `writeln!` |

## Agent Watcher Model

Each running agent gets one tokio task that monitors Claude's JSONL session
log via the `notify` crate (OS-level file events) with a fallback polling
loop:

```diagram
agent watcher task
│
├─ wait for session log to appear    (polls at OJ_SESSION_POLL_MS, max 30s)
├─ create notify::RecommendedWatcher (kqueue/inotify, shared thread)
│
└─ select! loop
     ├─ file_rx.recv()               parse log → emit AgentState event
     ├─ sleep(OJ_WATCHER_POLL_MS)    liveness check (default 5s)
     └─ shutdown_rx                  oneshot from agent kill
```

The `notify` crate runs a single internal thread shared across all watchers.
File change callbacks use `blocking_send()` to cross the sync-to-async
boundary.

## Known Blocking Paths

These are the paths where the event loop or IPC handlers are blocked for
extended periods in the current implementation:

### Event loop blocked by inline effects

The longest-blocking effects in `execute_all()`:

| Effect | What blocks | Worst case |
|--------|-------------|------------|
| `CreateWorkspace` | `git worktree add` subprocess | 5-30+s |
| `DeleteWorkspace` | `git worktree remove` + `remove_dir_all` | 5-30+s |
| `SpawnAgent` | tmux spawn + 3 prompt polls (200ms x 15 each) | 1-9s |
| `SendToAgent` | 2x Esc + literal send + settle sleep (1ms/char, cap 2s) | ~2.2s |
| `TakeQueueItem` | bash subprocess (same pattern as Shell but not spawned) | variable |

### Queries blocked by state lock

The state lock is **not** held across long `.await` points — it is acquired
and released in brief scoped blocks (during `apply_event()` in
`process_event()`, and again at several points within `execute_inner()`).
Query handlers can interleave between these brief acquisitions.

In practice, lock contention is low: the real issue is that subprocess-calling
IPC handlers (`PeekSession`, `WorkspacePrune`, `AgentResume`) block their
connection task on external process I/O for seconds, with no timeout.

### Cascading event chains

A single `CommandRun` produces result events (`WorkspaceReady`,
`SessionCreated`) that feed back through the WAL. Each result event triggers
another `process_event()` iteration that may execute more inline effects.
The full chain for a workspace+agent pipeline:

```diagram
CommandRun ─► CreateWorkspace (5-30s) ─► WorkspaceReady
  ─► SpawnAgent (1-9s) ─► SessionCreated ─► SetTimer (~µs)
```

Total: 6-39s of sequential blocking across multiple event iterations.

### Timer starvation

The `tokio::select!` timer branch only runs when the event reader branch
yields. During multi-second effect execution, the timer branch cannot fire.
Timer resolution degrades from the configured 1s interval to the duration of
the longest effect chain.

## See Also

- [Daemon](01-daemon.md) - Process architecture, lifecycle, IPC protocol
- [Effects](02-effects.md) - Effect types and execution
- [Storage](04-storage.md) - WAL and snapshot persistence
- [Adapters](05-adapters.md) - Integration adapters
