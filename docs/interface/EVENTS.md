# Events

Events provide observability and enable loose coupling between components.

## Wire Format

All events are serialized as flat JSON objects with a `type` field using `namespace:action` format and persisted to the WAL for crash recovery:

```json
{"type":"job:created","id":"p1","kind":"build","name":"test",...}
{"type":"agent:failed","agent_id":"a1","error":"RateLimited"}
{"type":"system:shutdown"}
```

## Type Tag Convention

Event origin distinguishes categories:
- **Signals** (bare verb/noun): Emitted **internally by the engine** to notify about things that happened. Examples: `command:run`, `timer:start`, `system:shutdown`, `agent:waiting`
- **State mutations** (past participle/adjective): `job:created`, `session:deleted`, `agent:working`, `step:started`
- **Actions** (imperative): Emitted **externally by the CLI or agents** to trigger runtime operations. Examples: `session:input`, `job:resume`, `job:cancel`, `agent:signal`

## Signal Events

Emitted **internally by the engine** to notify about things that happened. These do not affect `MaterializedState`:

| Type tag | Variant | Fields |
|----------|---------|--------|
| `command:run` | CommandRun | `job_id`, `job_name`, `project_root`, `invoke_dir`, `command`, `args` |
| `timer:start` | TimerStart | `id` |
| `agent:waiting` | AgentWaiting | `agent_id` |
| `system:shutdown` | Shutdown | _(none)_ |

`agent:waiting` is a no-op in `apply_event()` — the agent is idle but still running.

## State Mutation Events

Applied via `MaterializedState::apply_event()` to update in-memory state. All events (including signals) are persisted to WAL; this section lists those that actually mutate state.

### Job lifecycle

| Type tag | Variant | Fields |
|----------|---------|--------|
| `runbook:loaded` | RunbookLoaded | `hash`, `version`, `runbook` |
| `job:created` | JobCreated | `id`, `kind`, `name`, `runbook_hash`, `cwd`, `vars`, `initial_step`, `created_at_epoch_ms`, `namespace` |
| `job:advanced` | JobAdvanced | `id`, `step` |
| `job:updated` | JobUpdated | `id`, `vars` |
| `job:deleted` | JobDeleted | `id` |

### Step lifecycle

| Type tag | Variant | Fields |
|----------|---------|--------|
| `step:started` | StepStarted | `job_id`, `step`, `agent_id?` |
| `step:waiting` | StepWaiting | `job_id`, `step`, `reason?` |
| `step:completed` | StepCompleted | `job_id`, `step` |
| `step:failed` | StepFailed | `job_id`, `step`, `error` |
| `shell:exited` | ShellExited | `job_id`, `step`, `exit_code` |

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
| `session:created` | SessionCreated | `id`, `job_id` |
| `session:deleted` | SessionDeleted | `id` |
| `workspace:created` | WorkspaceCreated | `id`, `path`, `branch?`, `owner?`, `mode?` |
| `workspace:ready` | WorkspaceReady | `id` |
| `workspace:failed` | WorkspaceFailed | `id`, `reason` |
| `workspace:deleted` | WorkspaceDeleted | `id` |

### Cron lifecycle

| Type tag | Variant | Fields |
|----------|---------|--------|
| `cron:started` | CronStarted | `cron_name`, `project_root`, `runbook_hash`, `interval`, `job_name`, `namespace` |
| `cron:stopped` | CronStopped | `cron_name`, `namespace` |
| `cron:fired` | CronFired | `cron_name`, `job_id`, `namespace` |
| `cron:deleted` | CronDeleted | `cron_name`, `namespace` |

`cron:fired` is a tracking event — it does not mutate state directly (job creation is handled by `job:created`).

### Worker lifecycle

| Type tag | Variant | Fields |
|----------|---------|--------|
| `worker:started` | WorkerStarted | `worker_name`, `project_root`, `runbook_hash`, `queue_name`, `concurrency`, `namespace` |
| `worker:wake` | WorkerWake | `worker_name` |
| `worker:poll_complete` | WorkerPollComplete | `worker_name`, `items` |
| `worker:item_dispatched` | WorkerItemDispatched | `worker_name`, `item_id`, `job_id` |
| `worker:stopped` | WorkerStopped | `worker_name` |
| `worker:deleted` | WorkerDeleted | `worker_name`, `namespace` |

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
| `job:resume` | JobResume | `id`, `message?`, `vars` |
| `job:cancel` | JobCancel | `id` |
| `workspace:drop` | WorkspaceDrop | `id` |
| `agent:signal` | AgentSignal | `agent_id`, `kind`, `message?` |

Unlike other actions, `agent:signal` stores the signal on the job for the engine to act on; `kind` is `"complete"` (advance job) or `"escalate"` (pause and notify human).
