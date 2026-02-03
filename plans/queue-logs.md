# Queue Logs

## Overview

Add per-queue activity logs that record lifecycle events (pushed, dispatched, completed, failed, retried, dropped) and expose them via `oj queue logs <name>` with `-f/--follow`, `-n/--limit`, and `--project` flags. Follows the same architecture as agent logs: a path builder in `log_paths.rs`, a `QueueLogger` that writes timestamped lines, instrumentation at event emission sites, and a query/response path from daemon to CLI.

## Project Structure

Files to create:
```
crates/engine/src/queue_logger.rs        # QueueLogger (append-only writer)
crates/engine/src/queue_logger_tests.rs  # Unit tests for QueueLogger
```

Files to modify:
```
crates/engine/src/log_paths.rs           # Add queue_log_path()
crates/engine/src/log_paths_tests.rs     # Tests for queue_log_path()
crates/engine/src/lib.rs                 # Export queue_logger module
crates/engine/src/runtime/mod.rs         # Add QueueLogger to Runtime
crates/engine/src/runtime/handlers/mod.rs    # Instrument QueuePushed
crates/engine/src/runtime/handlers/worker.rs # Instrument QueueTaken, QueueCompleted, QueueFailed, QueueItemDead
crates/engine/src/runtime/handlers/timer.rs  # Instrument QueueItemRetry
crates/daemon/src/listener/queues.rs     # Instrument QueueDropped (from CLI drop command)
crates/daemon/src/protocol.rs            # Add GetQueueLogs query + QueueLogs response
crates/daemon/src/listener/query.rs      # Handle GetQueueLogs query
crates/cli/src/commands/queue.rs         # Add Logs subcommand
crates/cli/src/client.rs                 # Add get_queue_logs() method
```

## Dependencies

No new external dependencies. Uses existing `notify` crate (already a dependency for `tail_file` in `output.rs`), `std::fs`, and `std::io`.

## Implementation Phases

### Phase 1: Log Path Builder

Add `queue_log_path` to `crates/engine/src/log_paths.rs`:

```rust
/// Build the path to a queue's activity log file.
///
/// Structure: `{logs_dir}/queue/{queue_name}.log`
pub fn queue_log_path(logs_dir: &Path, queue_name: &str) -> PathBuf {
    logs_dir.join("queue").join(format!("{}.log", queue_name))
}
```

Add tests in `log_paths_tests.rs` following the existing pattern for `agent_log_path`.

**Verify:** `cargo test -p oj-engine log_paths`

### Phase 2: QueueLogger

Create `crates/engine/src/queue_logger.rs` following the `PipelineLogger` pattern (open-write-close per call, no channel — queue events are low frequency):

```rust
pub struct QueueLogger {
    log_dir: PathBuf,
}

impl QueueLogger {
    pub fn new(log_dir: PathBuf) -> Self { ... }

    /// Append a timestamped log line to the queue's log file.
    ///
    /// Format: `2026-01-30T08:14:09Z [item_id_prefix] message`
    ///
    /// Failures logged via tracing, never propagated.
    pub fn append(&self, queue_name: &str, item_id: &str, message: &str) { ... }
}
```

The `item_id` is included as a bracketed prefix (first 8 chars) so users can grep/filter by item. Log line format:

```
2026-02-03T10:15:00Z [a1b2c3d4] pushed data={url=https://example.com}
2026-02-03T10:15:01Z [a1b2c3d4] dispatched worker=my-worker
2026-02-03T10:15:30Z [a1b2c3d4] completed
2026-02-03T10:15:30Z [e5f6g7h8] failed error="timeout exceeded"
2026-02-03T10:16:00Z [e5f6g7h8] retried attempt=2
2026-02-03T10:17:00Z [e5f6g7h8] dead
2026-02-03T10:18:00Z [a1b2c3d4] dropped
```

Reuse the `format_utc_now()` helper from `pipeline_logger.rs` — extract it to a shared utility or duplicate it (it's small). Prefer extracting to a private `fn` in `queue_logger.rs` since the pipeline logger already has its own copy.

Export from `crates/engine/src/lib.rs`:
```rust
pub mod queue_logger;
```

Create `queue_logger_tests.rs` with unit tests covering each log entry type and the file creation/append behavior.

**Verify:** `cargo test -p oj-engine queue_logger`

### Phase 3: Instrumentation (Write Log Entries)

Wire the `QueueLogger` into the runtime and event handling code.

**3a. Add QueueLogger to Runtime** (`crates/engine/src/runtime/mod.rs`):

Add a `queue_logger: QueueLogger` field to the `Runtime` struct, initialized alongside `PipelineLogger`:
```rust
queue_logger: QueueLogger::new(config.log_dir.clone()),
```

**3b. Instrument event handlers:**

All instrumentation calls are fire-and-forget (logger handles errors internally).

| Event | File | Log message |
|-------|------|-------------|
| `QueuePushed` | `runtime/handlers/mod.rs:203` | `pushed data={key=val, ...}` |
| `QueueTaken` | `runtime/handlers/worker.rs:346` | `dispatched worker={worker_name}` |
| `QueueCompleted` | `runtime/handlers/worker.rs:505` | `completed` |
| `QueueFailed` | `runtime/handlers/worker.rs:511` | `failed error="{error}"` |
| `QueueItemDead` | `runtime/handlers/worker.rs:572` | `dead` |
| `QueueItemRetry` | `runtime/handlers/timer.rs:127` | `retried` |
| `QueueDropped` | `daemon/src/listener/queues.rs` (handle_queue_drop) | `dropped` |

For `QueueDropped`, the event is emitted in the daemon listener, not the engine runtime. The daemon already has access to `logs_path`. Instantiate a `QueueLogger` inline or pass `logs_path` to write the log entry directly in `handle_queue_drop`, after emitting the event. Same pattern: create the logger, call `append()`.

For the engine-side events, `self.queue_logger.append(...)` is called right after (or right before) the `Effect::Emit` for the corresponding event.

**Namespace-scoped queue names:** The log file should use the **scoped** queue name (i.e., `{namespace}/{queue_name}`) to avoid collisions across projects. The `queue_log_path` function receives the queue_name as-is; callers pass the scoped key. Adjust the path builder to handle `/` in the name by creating subdirectories:

```rust
// {logs_dir}/queue/{namespace}/{queue_name}.log  (when namespaced)
// {logs_dir}/queue/{queue_name}.log              (when no namespace)
pub fn queue_log_path(logs_dir: &Path, queue_name: &str) -> PathBuf {
    logs_dir.join("queue").join(format!("{}.log", queue_name))
}
```

Since the scoped key is `namespace/queue_name`, `Path::join` will naturally create `{logs_dir}/queue/namespace/queue_name.log`. The `QueueLogger::append` method calls `create_dir_all` on the parent directory, so nested paths work automatically.

**Verify:** `cargo test -p oj-engine` and `cargo test -p ojd`

### Phase 4: Protocol & Query Handler

**4a. Add query variant** (`crates/daemon/src/protocol.rs`):

```rust
// In Query enum:
GetQueueLogs {
    queue_name: String,
    #[serde(default)]
    namespace: String,
    /// Number of most recent lines to return (0 = all)
    lines: usize,
},
```

**4b. Add response variant** (`crates/daemon/src/protocol.rs`):

```rust
// In Response enum:
QueueLogs {
    log_path: PathBuf,
    content: String,
},
```

**4c. Handle query** (`crates/daemon/src/listener/query.rs`):

```rust
Query::GetQueueLogs { queue_name, namespace, lines } => {
    use oj_engine::log_paths::queue_log_path;

    let scoped = if namespace.is_empty() {
        queue_name.clone()
    } else {
        format!("{}/{}", namespace, queue_name)
    };
    let path = queue_log_path(logs_path, &scoped);
    let content = read_log_file(&path, lines);
    Response::QueueLogs { log_path: path, content }
}
```

**Verify:** `cargo test -p ojd`

### Phase 5: Client Method & CLI Command

**5a. Add client method** (`crates/cli/src/client.rs`):

```rust
/// Get queue activity logs
pub async fn get_queue_logs(
    &self,
    queue_name: &str,
    namespace: &str,
    lines: usize,
) -> Result<(PathBuf, String), ClientError> {
    let request = Request::Query {
        query: Query::GetQueueLogs {
            queue_name: queue_name.to_string(),
            namespace: namespace.to_string(),
            lines,
        },
    };
    match self.send(&request).await? {
        Response::QueueLogs { log_path, content } => Ok((log_path, content)),
        other => Self::reject(other),
    }
}
```

**5b. Add CLI subcommand** (`crates/cli/src/commands/queue.rs`):

```rust
/// View queue activity log
Logs {
    /// Queue name
    queue: String,
    /// Stream live activity (like tail -f)
    #[arg(long, short = 'f')]
    follow: bool,
    /// Number of recent lines to show (default: 50)
    #[arg(short = 'n', long, default_value = "50")]
    limit: usize,
    /// Project namespace override
    #[arg(long = "project")]
    project: Option<String>,
},
```

**5c. Handle in match arm:**

```rust
QueueCommand::Logs { queue, follow, limit, project } => {
    let effective_namespace = project
        .or_else(|| std::env::var("OJ_NAMESPACE").ok())
        .unwrap_or_else(|| namespace.to_string());

    let (log_path, content) = client
        .get_queue_logs(&queue, &effective_namespace, limit)
        .await?;
    display_log(&log_path, &content, follow, format, "queue", &queue).await?;
}
```

This reuses the existing `display_log` and `tail_file` functions from `output.rs`, which already handle `--follow` mode using file watching via the `notify` crate.

**Verify:** `cargo test -p oj-cli` and `cargo build --all`

### Phase 6: Final Verification

- `make check` (fmt, clippy, tests, build, audit, deny)
- Manual test: push items to a queue, watch `oj queue logs <name>` output, verify entries for each lifecycle event
- Manual test: `oj queue logs <name> -f` streams new entries in real time
- Manual test: `oj queue logs <name> -n 10` shows last 10 lines
- Manual test: `oj queue logs <name> --project foo` uses correct namespace

## Key Implementation Details

### Log File Per Queue (Not Per Item)

The instructions specify `{logs_dir}/queue/{queue_name}.log` — a single log file per queue. This is the right granularity: queue logs are an activity stream for the queue as a whole, not per-item detail logs. Users can grep by item_id prefix in the bracketed tag.

### Namespace Scoping

Queue names in the log path include the namespace prefix (e.g., `myproject/build-queue.log`). This follows the existing `scoped_key` pattern used throughout `MaterializedState`. The `queue_log_path` function simply joins the scoped name, and `Path::join` handles the `/` by creating nested directories.

### No New Dependencies

The implementation reuses:
- `notify` crate for `--follow` mode (already used by `tail_file`)
- `std::fs` for append-only file I/O
- `display_log` + `tail_file` from `output.rs` for CLI display

### Error Handling

All log writes are fire-and-forget with `tracing::warn` on failure, matching the pattern in `PipelineLogger` and `AgentLogger`. Logging must never break the engine or daemon.

### QueueDropped Instrumentation Location

`QueueDropped` is emitted in the daemon listener (`queues.rs`), not in the engine runtime. The daemon has `logs_path` available via the query handler's context. Create a `QueueLogger` inline (it's cheap — just a `PathBuf`) and call `append()` after emitting the event.

## Verification Plan

1. **Unit tests** — `queue_log_path` in `log_paths_tests.rs`, `QueueLogger` in `queue_logger_tests.rs`
2. **Protocol round-trip** — Add a test in `protocol_tests.rs` for `GetQueueLogs` serialization
3. **Integration** — `cargo test --all` passes
4. **Lint** — `cargo clippy --all-targets --all-features -- -D warnings`
5. **Format** — `cargo fmt --all -- --check`
6. **Full check** — `make check`
