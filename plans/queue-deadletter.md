# Queue Dead Letter Semantics

## Overview

Add dead letter semantics to persisted queues: configurable auto-retry with cooldown, a `Dead` terminal status for items that exhaust retries, and an `oj queue retry` command to manually resurrect dead/failed items. When `attempts = 0` (the default), failed items transition directly to `Dead` with no auto-retry.

## Project Structure

Files to create or modify:

```
crates/
├── runbook/src/
│   └── queue.rs              # Add RetryConfig struct to QueueDef
├── core/src/
│   ├── event.rs              # Add QueueItemRetry event
│   └── timer.rs              # Add TimerId::queue_retry factory
├── storage/src/
│   └── state.rs              # Add Dead status, failure_count field, apply QueueItemRetry
├── engine/src/runtime/
│   └── handlers/
│       ├── worker.rs         # Retry-or-dead logic on pipeline failure; handle retry timer
│       └── timer.rs          # Route queue-retry timers
├── daemon/src/
│   ├── protocol.rs           # Add QueueRetry request/response
│   └── listener/
│       ├── mod.rs            # Route QueueRetry request
│       └── queues.rs         # handle_queue_retry handler
└── cli/src/commands/
    └── queue.rs              # Add retry subcommand
```

## Dependencies

No new external dependencies. Uses existing infrastructure:
- `serde` for `RetryConfig` deserialization
- `oj_core::TimerId` / `Effect::SetTimer` for cooldown scheduling
- `oj_core::Clock` / `FakeClock` for deterministic timer tests

## Implementation Phases

### Phase 1: Data Model — RetryConfig, Dead status, failure_count

**Goal:** Extend the data model so retry configuration is parsed from runbooks and queue items track failure count and dead status.

1. **`crates/runbook/src/queue.rs`** — Add `RetryConfig`:
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct RetryConfig {
       #[serde(default)]
       pub attempts: u32,        // 0 = no auto-retry (default)
       #[serde(default = "default_cooldown")]
       pub cooldown: String,     // e.g. "30s", default "0s"
   }

   fn default_cooldown() -> String { "0s".into() }
   ```
   Add `pub retry: Option<RetryConfig>` to `QueueDef` (serde `default`).

2. **`crates/storage/src/state.rs`** — Add `Dead` variant to `QueueItemStatus`:
   ```rust
   pub enum QueueItemStatus {
       Pending,
       Active,
       Completed,
       Failed,
       Dead,
   }
   ```
   Add `failure_count: u32` to `QueueItem` (default 0). Update `Default` / construction sites.

3. **`crates/core/src/event.rs`** — Add `QueueItemRetry` event:
   ```rust
   Event::QueueItemRetry {
       queue_name: String,
       item_id: String,
       namespace: String,
   }
   ```

4. **`crates/storage/src/state.rs`** — Apply `QueueItemRetry`: set `status = Pending`, reset `failure_count = 0`, clear `worker_name`.

5. **`crates/storage/src/state.rs`** — Update `QueueFailed` handler: increment `failure_count` on the item.

**Verification:** Unit tests in `state_tests.rs` — apply `QueueFailed` and assert `failure_count` increments; apply `QueueItemRetry` and assert status resets to `Pending` with `failure_count = 0`; confirm `Dead` status serialization round-trips.

### Phase 2: Retry-or-Dead Logic in Worker Handler

**Goal:** When a pipeline fails for a persisted queue item, check the queue's retry config and either schedule a retry timer or mark the item dead.

1. **`crates/core/src/timer.rs`** — Add factory:
   ```rust
   impl TimerId {
       pub fn queue_retry(queue_name: &str, item_id: &str) -> Self {
           Self::new(format!("queue-retry:{}:{}", queue_name, item_id))
       }
       pub fn is_queue_retry(&self) -> bool {
           self.0.starts_with("queue-retry:")
       }
   }
   ```

2. **`crates/engine/src/runtime/handlers/worker.rs`** — In `check_worker_pipeline_complete`, after emitting `QueueFailed`, look up the queue's `RetryConfig` from the runbook. Read the item's updated `failure_count` from state. Decision:
   - If `retry.attempts > 0` and `failure_count < retry.attempts`: emit `Effect::SetTimer { id: TimerId::queue_retry(&queue_name, &item_id), duration }` using `monitor::parse_duration(&retry.cooldown)`.
   - Otherwise: emit `Effect::Emit { event: Event::QueueItemDead { ... } }` (new event, see below).

3. **`crates/core/src/event.rs`** — Add `QueueItemDead` event:
   ```rust
   Event::QueueItemDead {
       queue_name: String,
       item_id: String,
       namespace: String,
   }
   ```

4. **`crates/storage/src/state.rs`** — Apply `QueueItemDead`: set `status = Dead`.

5. **`crates/engine/src/runtime/handlers/timer.rs`** — Route `"queue-retry:*"` timers:
   ```rust
   if let Some(rest) = id_str.strip_prefix("queue-retry:") {
       return self.handle_queue_retry_timer(rest).await;
   }
   ```
   The handler parses `queue_name:item_id`, emits `QueueItemRetry` event, then emits `WorkerWake` for attached workers.

6. **`poll_persisted_queue`** — Already filters on `Pending` only, so `Dead` items are automatically skipped. No change needed.

**Verification:** Unit tests with `FakeClock`:
- Item fails with `attempts=3`: assert timer is set, not yet dead.
- Timer fires: assert item goes back to `Pending`.
- Item fails 3 times: assert item becomes `Dead`, no timer set.
- Item fails with `attempts=0` (default): assert item becomes `Dead` immediately.

### Phase 3: `oj queue retry` CLI Command

**Goal:** Manual retry command to resurrect `Dead` or `Failed` items.

1. **`crates/daemon/src/protocol.rs`** — Add request/response:
   ```rust
   Request::QueueRetry {
       project_root: PathBuf,
       namespace: String,
       queue_name: String,
       item_id: String,
   }
   Response::QueueRetried { queue_name: String, item_id: String }
   ```

2. **`crates/daemon/src/listener/mod.rs`** — Route `QueueRetry` to `queues::handle_queue_retry`.

3. **`crates/daemon/src/listener/queues.rs`** — `handle_queue_retry`:
   - Load runbook, validate queue exists and is persisted.
   - Look up item in state; validate status is `Dead` or `Failed`.
   - Emit `QueueItemRetry` event.
   - Call `wake_attached_workers` to trigger re-poll.
   - Return `Response::QueueRetried`.

4. **`crates/cli/src/commands/queue.rs`** — Add `Retry` variant to `QueueCommand`:
   ```rust
   Retry {
       queue: String,
       item_id: String,
       #[clap(long)]
       project: Option<String>,
   }
   ```
   Send `Request::QueueRetry`, display confirmation with 8-char ID prefix (matching `drop` pattern).

**Verification:** Integration-level test: push item, fail it, verify it's `Dead`, run `oj queue retry`, verify it's `Pending` again.

### Phase 4: Runbook Validation & Display Polish

**Goal:** Validate retry config at parse time; update list display for new statuses.

1. **`crates/runbook/src/parser.rs`** — In the persisted queue validation block, if `retry` is present:
   - Validate `cooldown` parses as a valid duration.
   - Validate `attempts` is reasonable (non-negative; u32 ensures this).
   - Reject `retry` on external queues.

2. **`crates/cli/src/commands/queue.rs`** — The `list` command already uses `format!("{:?}", status).to_lowercase()` which will produce `"dead"` for the new variant automatically. No change needed, but verify.

3. **`crates/daemon/src/listener/query.rs`** — `QueueItemSummary` already uses a `String` for status, so it handles `Dead` transparently. Verify `failure_count` is useful to expose — add it to `QueueItemSummary` if desired for observability.

**Verification:** Parse a runbook with `retry = { attempts = 3, cooldown = "30s" }` — assert it loads. Parse with `cooldown = "invalid"` — assert parse error. Run `oj queue list` with dead items — verify display.

### Phase 5: End-to-End Tests

**Goal:** Validate the full flow in integration tests.

1. **Auto-retry flow:** Push item → worker picks it up → pipeline fails → item retries after cooldown → eventually goes dead after N failures.
2. **No-retry flow (default):** Push item → pipeline fails → item immediately dead.
3. **Manual retry flow:** Item is dead → `oj queue retry` → item is pending → worker picks it up.
4. **Idempotency:** Retry on an already-pending item returns an error.
5. **Timer determinism:** Use `FakeClock` to advance time and verify cooldown behavior without real sleeps.

## Key Implementation Details

### Event Ordering

When a pipeline fails, the event sequence is:
1. `QueueFailed` (increments `failure_count`, sets `Failed`)
2. Either `QueueItemDead` (sets `Dead`) or `SetTimer` (schedules retry)
3. On timer fire: `QueueItemRetry` (sets `Pending`) + `WorkerWake`

This ordering matters because step 2 reads `failure_count` from state *after* step 1 has been applied. The worker handler must ensure `QueueFailed` is emitted and applied to state before reading `failure_count` to make the retry-or-dead decision.

Looking at `check_worker_pipeline_complete`: it already emits `QueueFailed` via `self.executor.execute_all(vec![Effect::Emit { event: queue_event }])` and awaits the result. The executor applies the event to state before returning. So reading `failure_count` from state after that `.await` will see the incremented value. This is the correct place to add the retry-or-dead logic.

### RetryConfig Resolution

The `RetryConfig` is read from the `QueueDef` in the runbook at the time of failure (not at push time). This means editing retry config in a runbook takes effect for subsequent failures without restarting workers. The runbook is already refreshed from disk in `check_worker_pipeline_complete` via `refresh_worker_runbook`.

### Default Behavior

When no `retry` block is specified, `QueueDef.retry` is `None`. Treat `None` the same as `RetryConfig { attempts: 0, cooldown: "0s" }` — failed items go directly to `Dead`. This is safe because it makes the new `Dead` status the only behavioral change for existing users (items that were previously stuck as `Failed` forever now become `Dead`, but both are terminal and skipped by polling).

### Migration / Backwards Compatibility

Existing `Failed` items in persisted state will remain `Failed` (not automatically migrated to `Dead`). The `QueueItem` struct gains a `failure_count: u32` field with `#[serde(default)]` so existing snapshots deserialize correctly with `failure_count = 0`. The new `Dead` variant in `QueueItemStatus` needs careful serde handling — use `#[serde(rename_all = "lowercase")]` or ensure the existing serialization approach (derived `Debug` format) handles it. Since `QueueItemStatus` is stored in WAL events via serde, add the variant and rely on `serde(default)` / forward-compatible deserialization.

### Timer ID Format

`queue-retry:{scoped_queue_name}:{item_id}` — uses the scoped key (with namespace prefix if applicable) so timers are unique per namespace.

## Verification Plan

1. **`make check`** — full CI: fmt, clippy, quench, test, build, audit, deny.
2. **Unit tests:**
   - `state_tests.rs`: `QueueFailed` increments `failure_count`; `QueueItemRetry` resets to pending; `QueueItemDead` sets dead status.
   - `timer_tests.rs`: `TimerId::queue_retry` factory and `is_queue_retry` predicate.
   - `worker_tests.rs` (or equivalent): retry-or-dead decision logic with `FakeClock`.
   - `parser_tests.rs`: `RetryConfig` parsing, validation of invalid cooldown strings.
3. **Integration tests:**
   - Push + fail + auto-retry + eventual dead letter (with `FakeClock`).
   - Push + fail + immediate dead (no retry config).
   - `oj queue retry` resurrects dead item.
   - `oj queue list` shows "dead" status correctly.
