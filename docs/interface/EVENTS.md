# Events

Events provide observability and enable loose coupling between components.

## Wire Format

All events are serialized as flat JSON objects with a `type` field using `namespace:action` format and persisted to the WAL for crash recovery:

```json
{"type":"pipeline:created","id":"p1","kind":"build","name":"test",...}
{"type":"agent:failed","agent_id":"a1","error":"RateLimited"}
{"type":"system:shutdown"}
```

## Type Tag Convention

Event origin distinguishes categories:
- **Signals** (bare verb/noun): Emitted **internally by the engine** to notify about things that happened. Examples: `command:run`, `timer:start`, `system:shutdown`, `agent:waiting`
- **State mutations** (past participle/adjective): `pipeline:created`, `session:deleted`, `agent:working`, `step:started`
- **Actions** (imperative): Emitted **externally by the CLI or agents** to trigger runtime operations. Examples: `session:input`, `pipeline:resume`, `pipeline:cancel`, `agent:signal`

## Signal Events

Emitted **internally by the engine** to notify about things that happened. These do not affect `MaterializedState`:

| Type tag | Variant | Fields |
|----------|---------|--------|
| `command:run` | CommandRun | `pipeline_id`, `pipeline_name`, `project_root`, `invoke_dir`, `command`, `args` |
| `timer:start` | TimerStart | `id` |
| `agent:waiting` | AgentWaiting | `agent_id` |
| `system:shutdown` | Shutdown | _(none)_ |

`agent:waiting` is a no-op in `apply_event()` — the agent is idle but still running.

## State Mutation Events

Applied via `MaterializedState::apply_event()` to update in-memory state. All events (including signals) are persisted to WAL; this section lists those that actually mutate state.

### Pipeline lifecycle

| Type tag | Variant | Fields |
|----------|---------|--------|
| `runbook:loaded` | RunbookLoaded | `hash`, `version`, `runbook` |
| `pipeline:created` | PipelineCreated | `id`, `kind`, `name`, `runbook_hash`, `cwd`, `vars`, `initial_step`, `created_at_epoch_ms`, `namespace` |
| `pipeline:advanced` | PipelineAdvanced | `id`, `step` |
| `pipeline:updated` | PipelineUpdated | `id`, `vars` |
| `pipeline:deleted` | PipelineDeleted | `id` |

### Step lifecycle

| Type tag | Variant | Fields |
|----------|---------|--------|
| `step:started` | StepStarted | `pipeline_id`, `step`, `agent_id?` |
| `step:waiting` | StepWaiting | `pipeline_id`, `step`, `reason?` |
| `step:completed` | StepCompleted | `pipeline_id`, `step` |
| `step:failed` | StepFailed | `pipeline_id`, `step`, `error` |
| `shell:exited` | ShellExited | `pipeline_id`, `step`, `exit_code` |

### Agent lifecycle

| Type tag | Variant | Fields |
|----------|---------|--------|
| `agent:working` | AgentWorking | `agent_id` |
| `agent:failed` | AgentFailed | `agent_id`, `error` |
| `agent:exited` | AgentExited | `agent_id`, `exit_code?` |
| `agent:gone` | AgentGone | `agent_id` |

### Session and workspace lifecycle

| Type tag | Variant | Fields |
|----------|---------|--------|
| `session:created` | SessionCreated | `id`, `pipeline_id` |
| `session:deleted` | SessionDeleted | `id` |
| `workspace:created` | WorkspaceCreated | `id`, `path`, `branch?`, `owner?`, `mode?` |
| `workspace:ready` | WorkspaceReady | `id` |
| `workspace:failed` | WorkspaceFailed | `id`, `reason` |
| `workspace:deleted` | WorkspaceDeleted | `id` |

### Worker lifecycle

| Type tag | Variant | Fields |
|----------|---------|--------|
| `worker:started` | WorkerStarted | `worker_name`, `project_root`, `runbook_hash`, `queue_name`, `concurrency`, `namespace` |
| `worker:wake` | WorkerWake | `worker_name` |
| `worker:poll_complete` | WorkerPollComplete | `worker_name`, `items` |
| `worker:item_dispatched` | WorkerItemDispatched | `worker_name`, `item_id`, `pipeline_id` |
| `worker:stopped` | WorkerStopped | `worker_name` |

`worker:wake` and `worker:poll_complete` do not mutate state — they are signals handled by the runtime to drive the poll/dispatch cycle.

### Queue lifecycle

| Type tag | Variant | Fields |
|----------|---------|--------|
| `queue:pushed` | QueuePushed | `queue_name`, `item_id`, `data`, `pushed_at_epoch_ms`, `namespace` |
| `queue:taken` | QueueTaken | `queue_name`, `item_id`, `worker_name`, `namespace` |
| `queue:completed` | QueueCompleted | `queue_name`, `item_id`, `namespace` |
| `queue:failed` | QueueFailed | `queue_name`, `item_id`, `error`, `namespace` |
| `queue:item_retry` | QueueItemRetry | `queue_name`, `item_id`, `namespace` |
| `queue:item_dead` | QueueItemDead | `queue_name`, `item_id`, `namespace` |

Queue events track the lifecycle of items in persisted queues. `queue:pushed` triggers a `worker:wake` for any worker watching the queue. The full item lifecycle is event-sourced: pushed → taken → completed/failed/dead. When a queue has retry configuration, failed items are automatically retried after a cooldown period. Items that exhaust their retry attempts transition to `dead` via `queue:item_dead`. Dead or failed items can be manually resurrected via `queue:item_retry`.

## Action Events

Action events trigger runtime operations. They are emitted **externally by the CLI or agents** and handled by the runtime. They do not mutate `MaterializedState`:

| Type tag | Variant | Fields |
|----------|---------|--------|
| `session:input` | SessionInput | `id`, `input` |
| `pipeline:resume` | PipelineResume | `id`, `message?`, `vars` |
| `pipeline:cancel` | PipelineCancel | `id` |
| `workspace:drop` | WorkspaceDrop | `id` |
| `agent:signal` | AgentSignal | `agent_id`, `kind`, `message?` |

Unlike other actions, `agent:signal` stores the signal on the pipeline for the engine to act on; `kind` is `"complete"` (advance pipeline) or `"escalate"` (pause and notify human).
