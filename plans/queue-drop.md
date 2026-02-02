# Queue Drop

## Overview

Add `oj queue drop <queue> <item-id>` command to remove items from a persisted queue. This allows clearing stale or stuck items (e.g. when the backing branch was already deleted) regardless of their current status.

## Project Structure

Files to modify:

```
crates/
├── core/src/event.rs              # Add QueueDropped event variant
├── daemon/src/
│   ├── protocol.rs                # Add QueueDrop request, QueueDropped response
│   ├── listener/
│   │   ├── mod.rs                 # Route QueueDrop to handler
│   │   └── queues.rs              # Implement handle_queue_drop
├── storage/src/state.rs           # Apply QueueDropped: remove item from queue_items
├── engine/src/runtime/handlers/
│   └── mod.rs                     # Add QueueDropped to no-op arm
└── cli/src/commands/queue.rs      # Add Drop subcommand
```

## Dependencies

No new dependencies required. Uses existing crates: `clap`, `serde`, `oj_core`, `oj_storage`, `oj_runbook`.

## Implementation Phases

### Phase 1: Event and Protocol

Add the event variant and request/response types.

**`crates/core/src/event.rs`** — Add `QueueDropped` variant after `QueueFailed`:

```rust
#[serde(rename = "queue:dropped")]
QueueDropped {
    queue_name: String,
    item_id: String,
    #[serde(default)]
    namespace: String,
},
```

Also add its `log_summary` arm (after the `QueueFailed` arm, ~line 516):

```rust
Event::QueueDropped {
    queue_name,
    item_id,
    ..
} => format!("{t} queue={queue_name} item={item_id}"),
```

**`crates/daemon/src/protocol.rs`** — Add request variant after `QueuePush` (~line 131):

```rust
/// Drop an item from a persisted queue
QueueDrop {
    project_root: PathBuf,
    #[serde(default)]
    namespace: String,
    queue_name: String,
    item_id: String,
},
```

Add response variant after `QueuePushed` (~line 273):

```rust
/// Item was dropped from queue
QueueDropped { queue_name: String, item_id: String },
```

### Phase 2: State Application

**`crates/storage/src/state.rs`** — Handle `QueueDropped` in `apply_event` after the `QueueFailed` arm (~line 585):

```rust
Event::QueueDropped {
    queue_name,
    item_id,
    namespace,
} => {
    let key = scoped_key(namespace, queue_name);
    if let Some(items) = self.queue_items.get_mut(&key) {
        items.retain(|i| i.id != *item_id);
    }
}
```

### Phase 3: Daemon Handler

**`crates/daemon/src/listener/queues.rs`** — Add `handle_queue_drop` function:

```rust
pub(super) fn handle_queue_drop(
    project_root: &Path,
    namespace: &str,
    queue_name: &str,
    item_id: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // 1. Load runbook, validate queue exists and is persisted
    //    (reuse load_runbook_for_queue + get_queue + queue_type check
    //     — same pattern as handle_queue_push lines 32-52)
    //
    // 2. Validate item exists in state:
    //    Lock state, look up scoped key, check item_id is present.
    //    Return Response::Error if not found.
    //
    // 3. Emit Event::QueueDropped
    //
    // 4. Return Response::QueueDropped { queue_name, item_id }
}
```

**`crates/daemon/src/listener/mod.rs`** — Route the new request after the `QueuePush` arm (~line 281):

```rust
Request::QueueDrop {
    project_root,
    namespace,
    queue_name,
    item_id,
} => queues::handle_queue_drop(
    &project_root,
    &namespace,
    &queue_name,
    &item_id,
    event_bus,
    state,
),
```

**`crates/engine/src/runtime/handlers/mod.rs`** — Add `QueueDropped` to the no-op arm alongside `QueueTaken`, `QueueCompleted`, `QueueFailed` (~line 202):

```rust
Event::QueueTaken { .. }
| Event::QueueCompleted { .. }
| Event::QueueFailed { .. }
| Event::QueueDropped { .. } => {}
```

### Phase 4: CLI Subcommand

**`crates/cli/src/commands/queue.rs`** — Add `Drop` variant to `QueueCommand`:

```rust
/// Remove an item from a persisted queue
Drop {
    /// Queue name
    queue: String,
    /// Item ID (or prefix)
    item_id: String,
    /// Project namespace override
    #[arg(long = "project")]
    project: Option<String>,
},
```

Add the handler arm in `handle()`:

```rust
QueueCommand::Drop {
    queue,
    item_id,
    project,
} => {
    let effective_namespace = project
        .or_else(|| std::env::var("OJ_NAMESPACE").ok())
        .unwrap_or_else(|| namespace.to_string());

    let request = Request::QueueDrop {
        project_root: project_root.to_path_buf(),
        namespace: effective_namespace,
        queue_name: queue.clone(),
        item_id: item_id.clone(),
    };

    match client.send(&request).await? {
        Response::QueueDropped { queue_name, item_id } => {
            println!("Dropped item {} from queue {}", &item_id[..8], queue_name);
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => {
            anyhow::bail!("unexpected response from daemon");
        }
    }
}
```

### Phase 5: Tests

**Unit test for state application** (`crates/storage/src/state_tests.rs`):
- Push an item, apply `QueueDropped`, verify it's removed from `queue_items`
- Apply `QueueDropped` for a non-existent item — verify it's a no-op (idempotent)

**Handler test** (`crates/daemon/src/listener/queues_tests.rs`):
- Push an item, then drop it — verify `QueueDropped` event is emitted and response is correct
- Drop from unknown queue — verify `Response::Error`
- Drop non-existent item — verify `Response::Error`

### Phase 6: Verification

Run `make check` to verify:
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `quench check`
- `cargo test --all`
- `cargo build --all`
- `cargo audit`
- `cargo deny check licenses bans sources`

## Key Implementation Details

1. **Namespace scoping**: Use the same `scoped_key(namespace, queue_name)` pattern from existing queue events to look up items in `MaterializedState::queue_items`.

2. **Item ID display**: Print only the first 8 characters of the item ID in CLI output (consistent with `oj queue list` at `queue.rs:149`).

3. **Validation order in handler**: Validate runbook/queue existence first (requires filesystem access), then validate item existence (requires state lock). This avoids holding the state lock longer than necessary.

4. **Idempotent state application**: `apply_event` for `QueueDropped` uses `retain` which is naturally idempotent — dropping a non-existent item is a no-op at the state level. Validation (error on missing item) happens in the handler, not in `apply_event`.

5. **No worker interaction**: Unlike `QueuePush`, dropping an item does not need to wake or notify workers. The item is simply removed from the persisted queue state.

## Verification Plan

1. **Compile** — `cargo build --all` passes with no errors
2. **Lint** — `cargo clippy --all-targets --all-features -- -D warnings` clean
3. **Format** — `cargo fmt --all -- --check` clean
4. **Unit tests** — New tests for state application and handler pass
5. **Existing tests** — `cargo test --all` passes (no regressions)
6. **Manual smoke test** — Push an item, verify it appears in `oj queue list`, drop it, verify it's gone
