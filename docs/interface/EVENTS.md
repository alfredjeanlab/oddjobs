# Events

Events provide observability and enable loose coupling between components.

## Wire Format

All events are serialized as flat JSON objects with a `type` field using `project:action` format and persisted to the WAL for crash recovery:

```json
{"type":"job:created","id":"p1","kind":"build","name":"test",...}
{"type":"agent:failed","agent_id":"a1","error":"RateLimited"}
{"type":"system:shutdown"}
```

## Type Tag Convention

Event origin distinguishes categories:
- **Signals** (bare verb/noun): Emitted **internally by the engine** to notify about things that happened. Examples: `command:run`, `timer:start`, `system:shutdown`, `agent:waiting`
- **State mutations** (past participle/adjective): `job:created`, `agent:spawned`, `agent:working`, `step:started`
- **Actions** (imperative): Emitted **externally by the CLI or agents** to trigger runtime operations. Examples: `agent:input`, `job:resume`, `job:cancel`, `agent:signal`

## Signal Events

Emitted **internally by the engine** to notify about things that happened. These do not affect `MaterializedState`:

- `command:run` — CLI command dispatched (creates job)
- `timer:start` — Scheduled timer fired
- `agent:waiting` — Agent idle but still running (no-op in state)
- `agent:idle` — Agent idle detected by coop (triggers idle grace timer)
- `agent:stop:blocked` — Agent tried to exit but stop gate blocked it
- `agent:prompt` — Agent showing a prompt (permission, plan, question)
- `system:shutdown` — Daemon shutting down

## State Mutation Events

Applied via `MaterializedState::apply_event()` to update in-memory state. All events (including signals) are persisted to WAL; this section lists those that actually mutate state.

### Job lifecycle

`job:created`, `job:advanced`, `job:updated`, `job:failing`, `job:cancelling`, `job:suspending`, `job:deleted`

`job:failing`, `job:cancelling`, and `job:suspending` are transitional states that mark the job as entering a terminal or suspended flow (e.g., triggering `on_fail`/`on_cancel` steps before the job reaches its final state).

### Step lifecycle

`step:started`, `step:waiting`, `step:completed`, `step:failed`, `shell:exited`

### Agent lifecycle

`agent:working`, `agent:failed`, `agent:exited`, `agent:gone`

All agent lifecycle events include an `owner` field (`OwnerId` — either a job or crew) for routing state changes to the correct owning entity.

### Spawn and workspace lifecycle

`agent:spawned`, `agent:spawn:failed`, `workspace:created`, `workspace:ready`, `workspace:failed`, `workspace:deleted`

### Cron lifecycle

`cron:started`, `cron:stopped`, `cron:once`, `cron:fired`, `cron:deleted`

`cron:once` triggers an immediate execution (ignoring interval). `cron:fired` is a tracking event — it does not mutate state directly (job creation is handled by `job:created`).

### Worker lifecycle

`worker:started`, `worker:wake`, `worker:polled`, `worker:took`, `worker:dispatched`, `worker:resized`, `worker:stopped`, `worker:deleted`

`worker:wake`, `worker:polled`, and `worker:took` do not mutate state — they are signals handled by the runtime to drive the poll/dispatch cycle. `worker:resized` updates the worker's concurrency configuration.

### Queue lifecycle

`queue:pushed`, `queue:taken`, `queue:completed`, `queue:failed`, `queue:dropped`, `queue:retry`, `queue:dead`

Queue events track the lifecycle of items in persisted queues. `queue:pushed` triggers a `worker:wake` for any worker watching the queue. The full item lifecycle is event-sourced: pushed → taken → completed/failed/dead. When a queue has retry configuration, failed items are automatically retried after a cooldown period. Items that exhaust their retry attempts transition to `dead` via `queue:dead`. Dead or failed items can be manually resurrected via `queue:retry`.

### Decision lifecycle

`decision:created`, `decision:resolved`

`decision:created` puts the owning job's step into `Waiting(decision_id)`. `decision:resolved` updates the decision record and emits a mapped action event (`job:resume`, `job:cancel`, `step:completed`, or `agent:input`). See [DECISIONS.md](DECISIONS.md) for sources, option mapping, and lifecycle.

### Crew

`crew:created`, `crew:started`, `crew:updated`, `crew:deleted`

crew are standalone agent executions (not tied to a job). They share the same lifecycle and monitoring as job-owned agents but are created directly via `command.run = { agent = "name" }`.

## Action Events

Action events trigger runtime operations. They are emitted **externally by the CLI or agents** and handled by the runtime:

- `agent:input` — Send freeform text to an agent
- `agent:respond` — Send a structured response to a prompt (plan approval, permission, question answer)
- `job:resume` — Resume a waiting/suspended job
- `job:cancel` — Cancel a job
- `job:suspend` — Suspend a running job
- `workspace:drop` — Delete a workspace
- `crew:resume` — Resume a waiting/suspended crew

These events do not mutate `MaterializedState` directly — the runtime handles them by emitting further state mutation events.
