# Plan: Cron Logs

## Overview

Add an `oj cron logs <name>` command that displays log output for a named cron, with support for `--follow`, `--limit`, and `--project` flags. This requires: a new `cron_log_path` helper, log writing instrumentation in the cron entrypoint, a new IPC query/response pair, a daemon query handler, a client method, and the CLI subcommand.

## Project Structure

Files to create or modify:

```
crates/engine/src/log_paths.rs          # Add cron_log_path()
crates/engine/src/log_paths_tests.rs    # Add test for cron_log_path()
crates/engine/src/runtime/handlers/cron.rs  # Add log writing on start/stop/tick/skip/error
crates/daemon/src/protocol.rs           # Add GetCronLogs query + CronLogs response
crates/daemon/src/listener/query.rs     # Add query handler
crates/cli/src/client.rs               # Add get_cron_logs() method
crates/cli/src/commands/cron.rs         # Add Logs subcommand + handler
```

## Dependencies

No new external dependencies. Uses existing `std::fs`, `std::io::Write`, `chrono` (if already available, otherwise use `std::time::SystemTime` for timestamps), and existing `display_log` helper from `crates/cli/src/output.rs`.

## Implementation Phases

### Phase 1: `cron_log_path` helper

Add to `crates/engine/src/log_paths.rs`:

```rust
/// Build the path to a cron log file.
///
/// Structure: `{logs_dir}/cron/{cron_name}.log`
pub fn cron_log_path(logs_dir: &Path, cron_name: &str) -> PathBuf {
    logs_dir.join("cron").join(format!("{}.log", cron_name))
}
```

Add test to `crates/engine/src/log_paths_tests.rs`:

```rust
#[test]
fn cron_log_path_builds_expected_path() {
    let result = cron_log_path(Path::new("/state/logs"), "nightly-deploy");
    assert_eq!(result, PathBuf::from("/state/logs/cron/nightly-deploy.log"));
}
```

**Verify:** `cargo test -p oj-engine log_paths`

### Phase 2: Cron log writing instrumentation

Add a helper function (private to the cron handler module or within the `Runtime` impl) that appends timestamped log lines to the cron log file. The function should create the `cron/` directory if it doesn't exist.

```rust
fn append_cron_log(logs_dir: &Path, cron_name: &str, message: &str) {
    let path = cron_log_path(logs_dir, cron_name);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let _ = writeln!(f, "[{}] {}", humantime::format_rfc3339_seconds(
            std::time::SystemTime::UNIX_EPOCH + now
        ), message);
    }
}
```

The `Runtime` needs access to `logs_dir`. Check if it's already available on the struct (it likely is, since agent log writing uses it). If not, it will need to be threaded through.

Instrument the following points in `crates/engine/src/runtime/handlers/cron.rs`:

1. **`handle_cron_started`** — after storing cron state and setting the timer:
   ```
   [2026-02-03T12:00:00Z] started (interval=5m, pipeline=deploy)
   ```

2. **`handle_cron_stopped`** — after updating status:
   ```
   [2026-02-03T12:05:00Z] stopped
   ```

3. **`handle_cron_timer_fired`** — after successfully creating and starting the pipeline:
   ```
   [2026-02-03T12:05:00Z] tick: triggered pipeline deploy (abc12345)
   ```

4. **`handle_cron_timer_fired`** — when cron is not running (early return):
   ```
   [2026-02-03T12:05:00Z] skip: cron not in running state
   ```

5. **`handle_cron_timer_fired`** and **`refresh_cron_runbook`** — on errors (runbook load failure, invalid interval, etc.):
   ```
   [2026-02-03T12:05:00Z] error: no runbook found containing cron 'nightly-deploy'
   ```

Note: For error logging, add the log write *before* returning the error, since the error propagates up. Consider wrapping error paths with a helper that logs then returns.

**Verify:** `cargo build -p oj-engine` compiles cleanly; manually start/stop a cron and confirm log file appears.

### Phase 3: IPC protocol additions

In `crates/daemon/src/protocol.rs`:

Add to `Query` enum:
```rust
GetCronLogs {
    /// Cron name
    name: String,
    /// Number of most recent lines to return (0 = all)
    lines: usize,
},
```

Add to `Response` enum:
```rust
/// Cron log contents
CronLogs {
    /// Path to the log file (for --follow mode)
    log_path: PathBuf,
    /// Log content (most recent N lines)
    content: String,
},
```

**Verify:** `cargo build -p oj-daemon` compiles (will have an unmatched arm warning in query handler until Phase 4).

### Phase 4: Daemon query handler

In `crates/daemon/src/listener/query.rs`, add a match arm for `GetCronLogs`:

```rust
Query::GetCronLogs { name, lines } => {
    use oj_engine::log_paths::cron_log_path;

    let log_path = cron_log_path(logs_path, &name);
    let content = match std::fs::read_to_string(&log_path) {
        Ok(text) => {
            if lines > 0 {
                let all_lines: Vec<&str> = text.lines().collect();
                let start = all_lines.len().saturating_sub(lines);
                all_lines[start..].join("\n")
            } else {
                text
            }
        }
        Err(_) => String::new(),
    };
    Response::CronLogs { log_path, content }
}
```

This follows the exact same pattern as `GetPipelineLogs` (see `query.rs:268-286`).

**Verify:** `cargo build -p oj-daemon`

### Phase 5: Client method

In `crates/cli/src/client.rs`, add:

```rust
/// Get cron logs
pub async fn get_cron_logs(
    &self,
    name: &str,
    lines: usize,
) -> Result<(PathBuf, String), ClientError> {
    let request = Request::Query {
        query: Query::GetCronLogs {
            name: name.to_string(),
            lines,
        },
    };
    match self.send(&request).await? {
        Response::CronLogs { log_path, content } => Ok((log_path, content)),
        other => Self::reject(other),
    }
}
```

**Verify:** `cargo build -p oj-cli`

### Phase 6: CLI subcommand

In `crates/cli/src/commands/cron.rs`:

Add `Logs` variant to `CronCommand`:

```rust
/// View cron activity log
Logs {
    /// Cron name from runbook
    name: String,
    /// Stream live activity (like tail -f)
    #[arg(long, short)]
    follow: bool,
    /// Number of recent lines to show (default: 50)
    #[arg(short = 'n', long, default_value = "50")]
    limit: usize,
    /// Project namespace override
    #[arg(long)]
    project: Option<String>,
},
```

Add the match arm in the `handle` function:

```rust
CronCommand::Logs { name, follow, limit, project } => {
    // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
    let effective_namespace = project
        .or_else(|| std::env::var("OJ_NAMESPACE").ok())
        .unwrap_or_else(|| namespace.to_string());
    // Cron logs are keyed by name, not namespace-scoped in path (yet),
    // but --project may be used for future namespace-scoped cron names.
    let (log_path, content) = client.get_cron_logs(&name, limit).await?;
    display_log(&log_path, &content, follow, format, "cron", &name).await?;
}
```

Add the `display_log` import from `crate::output::display_log` (check if already imported).

**Verify:** `cargo build -p oj-cli && oj cron logs --help`

## Key Implementation Details

- **Log format**: Use `[RFC 3339 timestamp] message` format (e.g., `[2026-02-03T12:00:00Z] tick: triggered pipeline deploy (abc12345)`). Use `humantime` if available in the workspace, otherwise `SystemTime` formatting.
- **Log path**: `{logs_dir}/cron/{cron_name}.log` — one file per cron name, appended over time.
- **Directory creation**: The `append_cron_log` helper must create `{logs_dir}/cron/` on first write.
- **Errors in log writing are silently ignored** (same pattern as tracing — logging should not cause the cron to fail).
- **`--follow` mode** works via `display_log` which already handles tailing the file path returned.
- **`--project` flag** follows the same namespace resolution pattern as `oj worker start --project` and `oj queue push --project` (see `crates/cli/src/commands/worker.rs:61`). Currently cron log paths are not namespace-scoped, but the flag is accepted for consistency and forward-compatibility.
- **`logs_dir` access in Runtime**: The Runtime struct needs access to the logs directory path. Check if `self.logs_dir` or equivalent already exists; if not, add it to the Runtime struct and thread it through construction.

## Verification Plan

1. **Unit tests**: `cron_log_path` test in `log_paths_tests.rs`
2. **Build check**: `cargo build --all` passes
3. **Lint check**: `cargo clippy --all-targets --all-features -- -D warnings`
4. **Format check**: `cargo fmt --all -- --check`
5. **Full test suite**: `cargo test --all`
6. **Manual test**: Start a cron (`oj cron start <name>`), wait for a tick, then `oj cron logs <name>` and `oj cron logs <name> -f`
7. **`make check`** passes
