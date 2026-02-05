# Storage Layer

Write-ahead log (WAL) for durable state persistence with crash recovery.

## Architecture

```diagram
Event → WAL (append + fsync) → Materialized State
                  ↓
            Snapshots (periodic)
```

State is derived from WAL. On startup, load latest snapshot then replay WAL entries.

## WAL Entry Format

JSONL format — one JSON object per line:

```
{"seq":1,"event":{"type":"job:created","id":"p1","kind":"build",...}}\n
{"seq":2,"event":{"type":"step:completed","job_id":"p1","step":"build"}}\n
```

- **seq**: Monotonic sequence number, never repeats
- **event**: JSON-serialized `Event` from oj-core (tagged via `{"type": "event:name", ...fields}`)

The WAL stores core `Event` values directly. State mutations use typed `Event` variants (e.g., `JobCreated`, `StepFailed`) emitted via `Effect::Emit`.

### Group Commit

Writes are buffered in memory and flushed to disk either:
- On interval (~10ms)
- When the buffer reaches 100 entries
- Explicitly via `flush()`

A single `fsync` covers the entire batch for performance.

## State Mutation Events

State mutations use typed `Event` variants emitted via `Effect::Emit`. These events are written to WAL and applied by `MaterializedState::apply_event()`:

| Type Tag | Variant | Fields | Effect |
|---|---|---|---|
| `job:created` | JobCreated | id, kind, name, vars, runbook_hash, cwd, initial_step, created_at_epoch_ms, namespace | Insert job |
| `job:advanced` | JobAdvanced | id, step | Finalize current step, advance job |
| `job:deleted` | JobDeleted | id | Remove job |
| `job:updated` | JobUpdated | id, vars | Merge new vars into job |
| `step:started` | StepStarted | job_id, step, agent_id? | Mark step running, set agent_id |
| `step:waiting` | StepWaiting | job_id, step, reason? | Mark step waiting for intervention |
| `step:completed` | StepCompleted | job_id, step | Mark step completed |
| `step:failed` | StepFailed | job_id, step, error | Mark step failed with error |
| `runbook:loaded` | RunbookLoaded | hash, version, runbook | Cache runbook by content hash (dedup) |
| `session:created` | SessionCreated | id, job_id | Insert session, link to job |
| `session:deleted` | SessionDeleted | id | Remove session |
| `workspace:created` | WorkspaceCreated | id, path, branch, owner, mode | Insert workspace (status=Creating), link to job |
| `workspace:ready` | WorkspaceReady | id | Set workspace status to Ready |
| `workspace:failed` | WorkspaceFailed | id, reason | Set workspace status to Failed |
| `workspace:deleted` | WorkspaceDeleted | id | Remove workspace |

Agent signal and lifecycle events also update job status during replay:

| Type Tag | Variant | Fields | Effect |
|---|---|---|---|
| `agent:working` | AgentWorking | agent_id | Set job step_status to Running |
| `agent:waiting` | AgentWaiting | agent_id | No state change (agent idle but alive) |
| `agent:exited` | AgentExited | agent_id, exit_code | Set job Completed (exit 0) or Failed |
| `agent:failed` | AgentFailed | agent_id, error | Set job step_status to Failed |
| `agent:gone` | AgentGone | agent_id | Set job Failed (session terminated) |
| `agent:signal` | AgentSignal | agent_id, kind, message? | Set job agent_signal |
| `shell:exited` | ShellExited | job_id, step, exit_code | Finalize step as Completed (0) or Failed |

### Worker and queue lifecycle

| Type Tag | Variant | Fields | Effect |
|---|---|---|---|
| `worker:started` | WorkerStarted | worker_name, project_root, runbook_hash, queue_name, concurrency, namespace | Insert or update worker record |
| `worker:item_dispatched` | WorkerItemDispatched | worker_name, item_id, job_id | Track dispatched item on worker |
| `worker:stopped` | WorkerStopped | worker_name | Remove worker record |
| `queue:pushed` | QueuePushed | queue_name, item_id, data, pushed_at_epoch_ms, namespace | Insert queue item (status=Pending) |
| `queue:taken` | QueueTaken | queue_name, item_id, worker_name, namespace | Set queue item status to Taken |
| `queue:completed` | QueueCompleted | queue_name, item_id, namespace | Remove queue item |
| `queue:failed` | QueueFailed | queue_name, item_id, error, namespace | Set queue item status to Failed, increment failure_count |
| `queue:item_retry` | QueueItemRetry | queue_name, item_id, namespace | Reset item to Pending, clear failure_count |
| `queue:item_dead` | QueueItemDead | queue_name, item_id, namespace | Set queue item status to Dead (terminal) |

### Cron lifecycle

| Type Tag | Variant | Fields | Effect |
|---|---|---|---|
| `cron:started` | CronStarted | cron_name, project_root, runbook_hash, interval, job_name, run_target, namespace | Insert or update cron record |
| `cron:stopped` | CronStopped | cron_name, namespace | Set cron status to stopped |
| `cron:fired` | CronFired | cron_name, namespace, job_id? | Update last_fired_at_ms |
| `cron:deleted` | CronDeleted | cron_name, namespace | Remove cron record |

### Decision lifecycle

| Type Tag | Variant | Fields | Effect |
|---|---|---|---|
| `decision:created` | DecisionCreated | id, job_id, agent_id?, source, context, options, created_at_ms, namespace | Insert decision, set job to Waiting |
| `decision:resolved` | DecisionResolved | id, chosen?, message?, resolved_at_ms | Update decision resolution |

### Standalone agent runs

| Type Tag | Variant | Fields | Effect |
|---|---|---|---|
| `agent_run:created` | AgentRunCreated | id, agent_name, command_name, namespace, cwd, runbook_hash, vars, created_at_epoch_ms | Insert agent run |
| `agent_run:started` | AgentRunStarted | id, agent_id | Set status to Running, link agent_id |
| `agent_run:status_changed` | AgentRunStatusChanged | id, status, reason? | Update status |
| `agent_run:deleted` | AgentRunDeleted | id | Remove agent run |

Action/signal events (`CommandRun`, `TimerStart`, `SessionInput`, `JobResume`, `JobCancel`, `WorkspaceDrop`, `Shutdown`, `Custom`) do not affect persisted state. `WorkerWake` and `WorkerPollComplete` are also signals that do not mutate state.

## Materialized State

State is rebuilt by replaying events:

```rust
pub struct MaterializedState {
    pub jobs: HashMap<String, Job>,
    pub sessions: HashMap<String, Session>,
    pub workspaces: HashMap<String, Workspace>,
    pub runbooks: HashMap<String, StoredRunbook>,
    pub workers: HashMap<String, WorkerRecord>,
    pub queue_items: HashMap<String, Vec<QueueItem>>,
    pub crons: HashMap<String, CronRecord>,
    pub decisions: HashMap<String, Decision>,
    pub agent_runs: HashMap<String, AgentRun>,
}
```

Each event type has logic in `apply_event()` that updates state deterministically.

Workers and queues use namespace-scoped composite keys (`namespace/name`) so that multiple projects can define identically-named resources without collision. When the namespace is empty (single-project setups), the bare name is used for backward compatibility.

## Snapshots

Periodic snapshots compress history:

```rust
pub struct Snapshot {
    pub version: u32,                // Schema version for migrations
    pub seq: u64,                    // WAL sequence at snapshot time
    pub state: MaterializedState,
    pub created_at: DateTime<Utc>,
}
```

Recovery: Load snapshot, migrate if needed, replay only entries after `snapshot.seq`.

### Checkpoint Flow

Checkpoints run every 60 seconds with I/O off the main thread:

```diagram
Main Thread (async)           Background Thread
─────────────────────────     ─────────────────────────────
clone state (~10ms)
  │
  └─────────────────────────→ serialize JSON (~100ms)
                              compress with zstd (~30ms)
                              write to .tmp (~20ms)
                              fsync .tmp (~50ms)
                              rename → snapshot.json (~1ms)
                              fsync directory (~30ms)
                                │
  ←─────────────────────────────┘ (completion signal)
truncate WAL (safe now)
```

**Critical invariant**: WAL truncation only happens after directory fsync. This ensures the snapshot rename survives power loss — without it, a crash could leave the old snapshot on disk while the WAL has been truncated, losing events.

### Compression

Snapshots use zstd compression (level 3) for ~70-80% size reduction:

| State size | JSON | zstd |
|------------|------|------|
| 100 jobs | ~1MB | ~200KB |
| 1000 jobs | ~10MB | ~2MB |

Snapshots are always zstd-compressed; the loader expects compressed format.

### Testability

The `CheckpointWriter` trait abstracts all I/O:

```rust
pub trait CheckpointWriter: Send + Sync + 'static {
    fn write_tmp(&self, path: &Path, data: &[u8]) -> Result<(), CheckpointError>;
    fn fsync_file(&self, path: &Path) -> Result<(), CheckpointError>;
    fn rename(&self, from: &Path, to: &Path) -> Result<(), CheckpointError>;
    fn fsync_dir(&self, path: &Path) -> Result<(), CheckpointError>;
    fn file_size(&self, path: &Path) -> Result<u64, CheckpointError>;
}
```

Tests use `FakeCheckpointWriter` to:
- Verify I/O ordering (fsync before rename, dir fsync before WAL truncation)
- Inject failures at any step
- Test crash recovery scenarios without touching the filesystem

### Versioning and Migrations

Snapshots include a schema version (`v` field in JSON). On load:

1. Parse JSON into `serde_json::Value`
2. Check version field
3. Apply migrations sequentially until current version
4. Deserialize to typed `Snapshot` struct

```
v1 → v2 → ... → current
```

Migrations transform JSON in place, allowing schema evolution without maintaining legacy Rust types. Each migration is a function `fn(&mut Value) -> Result<(), MigrationError>`.

**Why migrations are required**: WAL is truncated after checkpoint, so "discard snapshot and replay WAL" would lose all state before the snapshot. Migrations must succeed or the daemon fails to start.

| Scenario | Behavior |
|----------|----------|
| Old snapshot, new daemon | Migrate forward, load normally |
| New snapshot, old daemon | Fail with `MigrationError::TooNew` |
| Migration failure | Daemon startup fails (no silent data loss) |

## Compaction

On each checkpoint (every 60 seconds):
1. Take snapshot at current processed sequence (overwrites previous snapshot)
2. Rewrite WAL keeping only entries >= snapshot sequence (write to `.tmp`, fsync, atomic rename)

## Corruption Handling

| Problem | Detection | Recovery |
|---------|-----------|----------|
| Corrupt WAL entry | JSON parse fails during scan | Rotate WAL to `.bak`, preserve valid entries before corruption in a new clean WAL |
| Corrupt WAL (read) | JSON parse fails in `next_unprocessed` | Log warning, skip corrupt line, advance read offset |
| Corrupt snapshot | JSON parse fails on load | Move snapshot to `.bak`, recover via full WAL replay |
| Invalid UTF-8 in WAL | `InvalidData` IO error | Stop reading at corruption point |

Backup rotation keeps up to 3 `.bak` files (`.bak`, `.bak.2`, `.bak.3`), removing the oldest when the limit is reached.

## Invariants

- Flush (with fsync) is the durability point -- buffered writes are not durable until flushed
- Sequence numbers are monotonically increasing and never repeat
- Replaying same entries produces identical state
