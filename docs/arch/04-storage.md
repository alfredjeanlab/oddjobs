# Storage Layer

Write-ahead log (WAL) for durable state persistence with crash recovery.

## Architecture

```
Event → WAL (append + fsync) → Materialized State
                  ↓
            Snapshots (periodic)
```

State is derived from WAL. On startup, load latest snapshot then replay WAL entries.

## WAL Entry Format

JSONL format — one JSON object per line:

```
{"seq":1,"event":{"type":"pipeline:created","id":"p1","kind":"build",...}}\n
{"seq":2,"event":{"type":"step:completed","pipeline_id":"p1","step":"build"}}\n
```

- **seq**: Monotonic sequence number, never repeats
- **event**: JSON-serialized `Event` from oj-core (tagged via `{"type": "event:name", ...fields}`)

The WAL stores core `Event` values directly. State mutations use typed `Event` variants (e.g., `PipelineCreated`, `StepFailed`) emitted via `Effect::Emit`.

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
| `pipeline:created` | PipelineCreated | id, kind, name, vars, runbook_hash, cwd, initial_step, created_at_epoch_ms, namespace | Insert pipeline |
| `pipeline:advanced` | PipelineAdvanced | id, step | Finalize current step, advance pipeline |
| `pipeline:deleted` | PipelineDeleted | id | Remove pipeline |
| `pipeline:updated` | PipelineUpdated | id, vars | Merge new vars into pipeline |
| `step:started` | StepStarted | pipeline_id, step, agent_id? | Mark step running, set agent_id |
| `step:waiting` | StepWaiting | pipeline_id, step, reason? | Mark step waiting for intervention |
| `step:completed` | StepCompleted | pipeline_id, step | Mark step completed |
| `step:failed` | StepFailed | pipeline_id, step, error | Mark step failed with error |
| `runbook:loaded` | RunbookLoaded | hash, version, runbook | Cache runbook by content hash (dedup) |
| `session:created` | SessionCreated | id, pipeline_id | Insert session, link to pipeline |
| `session:deleted` | SessionDeleted | id | Remove session |
| `workspace:created` | WorkspaceCreated | id, path, branch, owner, mode | Insert workspace (status=Creating), link to pipeline |
| `workspace:ready` | WorkspaceReady | id | Set workspace status to Ready |
| `workspace:failed` | WorkspaceFailed | id, reason | Set workspace status to Failed |
| `workspace:deleted` | WorkspaceDeleted | id | Remove workspace |

Agent signal and lifecycle events also update pipeline status during replay:

| Type Tag | Variant | Fields | Effect |
|---|---|---|---|
| `agent:working` | AgentWorking | agent_id | Set pipeline step_status to Running |
| `agent:waiting` | AgentWaiting | agent_id | No state change (agent idle but alive) |
| `agent:exited` | AgentExited | agent_id, exit_code | Set pipeline Completed (exit 0) or Failed |
| `agent:failed` | AgentFailed | agent_id, error | Set pipeline step_status to Failed |
| `agent:gone` | AgentGone | agent_id | Set pipeline Failed (session terminated) |
| `agent:signal` | AgentSignal | agent_id, kind, message? | Set pipeline agent_signal |
| `shell:exited` | ShellExited | pipeline_id, step, exit_code | Finalize step as Completed (0) or Failed |

### Worker and queue lifecycle

| Type Tag | Variant | Fields | Effect |
|---|---|---|---|
| `worker:started` | WorkerStarted | worker_name, project_root, runbook_hash, queue_name, concurrency, namespace | Insert or update worker record |
| `worker:item_dispatched` | WorkerItemDispatched | worker_name, item_id, pipeline_id | Track dispatched item on worker |
| `worker:stopped` | WorkerStopped | worker_name | Remove worker record |
| `queue:pushed` | QueuePushed | queue_name, item_id, data, pushed_at_epoch_ms, namespace | Insert queue item (status=Pending) |
| `queue:taken` | QueueTaken | queue_name, item_id, worker_name, namespace | Set queue item status to Taken |
| `queue:completed` | QueueCompleted | queue_name, item_id, namespace | Remove queue item |
| `queue:failed` | QueueFailed | queue_name, item_id, error, namespace | Set queue item status to Failed, increment failure_count |
| `queue:item_retry` | QueueItemRetry | queue_name, item_id, namespace | Reset item to Pending, clear failure_count |
| `queue:item_dead` | QueueItemDead | queue_name, item_id, namespace | Set queue item status to Dead (terminal) |

Action/signal events (`CommandRun`, `TimerStart`, `SessionInput`, `PipelineResume`, `PipelineCancel`, `WorkspaceDrop`, `Shutdown`, `Custom`) do not affect persisted state. `WorkerWake` and `WorkerPollComplete` are also signals that do not mutate state.

## Materialized State

State is rebuilt by replaying events:

```rust
pub struct MaterializedState {
    pub pipelines: HashMap<String, Pipeline>,
    pub sessions: HashMap<String, Session>,
    pub workspaces: HashMap<String, Workspace>,
    pub runbooks: HashMap<String, StoredRunbook>,
    pub workers: HashMap<String, WorkerRecord>,
    pub queue_items: HashMap<String, Vec<QueueItem>>,
}
```

Each event type has logic in `apply_event()` that updates state deterministically.

Workers and queues use namespace-scoped composite keys (`namespace/name`) so that multiple projects can define identically-named resources without collision. When the namespace is empty (single-project setups), the bare name is used for backward compatibility.

## Snapshots

Periodic snapshots compress history:

```rust
pub struct Snapshot {
    pub seq: u64,                    // WAL sequence at snapshot time
    pub state: MaterializedState,
    pub created_at: DateTime<Utc>,
}
```

Recovery: Load snapshot, replay only entries after `snapshot.seq`.

Snapshots are saved atomically (write to temp file, fsync, rename). Checkpoints run every 60 seconds: save a snapshot, then truncate the WAL to only entries at or after the snapshot sequence.

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
