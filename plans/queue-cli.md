# Plan: Queue CLI Refactor

## Overview

Refactor `oj queue list` from showing items in a single queue to showing all known queues with metadata (type, item count, worker info). Add a new `oj queue items <queue-name>` subcommand that replaces the current item-listing behavior. Remove the `--queue` flag from `list` entirely.

## Project Structure

Key files to modify:

```
crates/
├── cli/src/commands/queue.rs        # CLI arg parsing + display logic
├── daemon/src/protocol.rs           # Query/Response types (add ListQueues, QueueSummary)
├── daemon/src/listener/query.rs     # Query handler (implement ListQueues)
├── runbook/src/find.rs              # Add collect_all_queues() helper
├── runbook/src/lib.rs               # Re-export collect_all_queues
docs/
└── interface/CLI.md                 # Update CLI docs
```

## Dependencies

No new external dependencies. Uses existing `oj_runbook`, `oj_daemon`, `clap`, and `serde` crates.

## Implementation Phases

### Phase 1: Add `collect_all_queues()` to `oj_runbook`

Add a function in `crates/runbook/src/find.rs` that scans all runbook files and collects every queue definition, similar to the existing `collect_all_commands()` function.

**Files:** `crates/runbook/src/find.rs`, `crates/runbook/src/lib.rs`

**Changes:**
1. Add `collect_all_queues()` in `find.rs`:
   ```rust
   /// Scan `.oj/runbooks/` and collect all queue definitions.
   /// Returns a sorted vec of (queue_name, QueueDef) pairs.
   /// Skips runbooks that fail to parse.
   pub fn collect_all_queues(
       runbook_dir: &Path,
   ) -> Result<Vec<(String, crate::QueueDef)>, FindError> {
       if !runbook_dir.exists() {
           return Ok(Vec::new());
       }
       let files = collect_runbook_files(runbook_dir)?;
       let mut queues = Vec::new();
       for (path, format) in files {
           let content = match std::fs::read_to_string(&path) {
               Ok(c) => c,
               Err(e) => {
                   tracing::warn!(path = %path.display(), error = %e, "skipping unreadable runbook");
                   continue;
               }
           };
           let runbook = match parse_runbook_with_format(&content, format) {
               Ok(rb) => rb,
               Err(e) => {
                   tracing::warn!(path = %path.display(), error = %e, "skipping invalid runbook");
                   continue;
               }
           };
           for (name, queue) in runbook.queues {
               queues.push((name, queue));
           }
       }
       queues.sort_by(|a, b| a.0.cmp(&b.0));
       Ok(queues)
   }
   ```
2. Re-export `collect_all_queues` from `lib.rs` (add to the existing `pub use find::` line).

**Verify:** `cargo test -p oj-runbook` passes. Consider adding a test similar to `collect_all_commands` tests if they exist.

### Phase 2: Add `ListQueues` query and `QueueSummary` response type

Add protocol types for the new query that returns all queues with metadata.

**Files:** `crates/daemon/src/protocol.rs`

**Changes:**
1. Add `QueueSummary` struct:
   ```rust
   /// Summary of a queue for listing
   #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
   pub struct QueueSummary {
       pub name: String,
       /// "persisted" or "external"
       pub queue_type: String,
       /// Number of items (persisted queues only; 0 for external)
       pub item_count: usize,
       /// Workers attached to this queue
       pub workers: Vec<String>,
   }
   ```
2. Add `ListQueues` variant to the `Query` enum:
   ```rust
   /// List all known queues in a project
   ListQueues {
       project_root: PathBuf,
       #[serde(default)]
       namespace: String,
   },
   ```
   Note: `ListQueues` needs `project_root` (unlike other queries) because it must scan runbook files to discover queue definitions. This is consistent with how mutation handlers like `handle_queue_push` take `project_root`.
3. Add `Queues` variant to the `Response` enum:
   ```rust
   Queues { queues: Vec<QueueSummary> },
   ```

**Verify:** `cargo test -p oj-daemon` passes (protocol encode/decode tests).

### Phase 3: Implement `ListQueues` query handler

Wire up the daemon to handle the new query by scanning runbooks for queue definitions and enriching with state data.

**Files:** `crates/daemon/src/listener/query.rs`

**Changes:**
1. Import `QueueSummary` and `collect_all_queues`.
2. Add handler for `Query::ListQueues` in the match block:
   ```rust
   Query::ListQueues { project_root, namespace } => {
       let runbook_dir = project_root.join(".oj/runbooks");
       let queue_defs = oj_runbook::collect_all_queues(&runbook_dir)
           .unwrap_or_default();

       let queues = queue_defs.into_iter().map(|(name, def)| {
           // Count persisted items from state
           let key = if namespace.is_empty() {
               name.clone()
           } else {
               format!("{}/{}", namespace, name)
           };
           let item_count = state.queue_items
               .get(&key)
               .map(|items| items.len())
               .unwrap_or(0);

           // Find workers attached to this queue
           let workers: Vec<String> = state.workers.values()
               .filter(|w| w.queue_name == name && w.namespace == namespace)
               .map(|w| w.name.clone())
               .collect();

           let queue_type = match def.queue_type {
               oj_runbook::QueueType::External => "external",
               oj_runbook::QueueType::Persisted => "persisted",
           };

           QueueSummary {
               name,
               queue_type: queue_type.to_string(),
               item_count,
               workers,
           }
       }).collect();

       Response::Queues { queues }
   }
   ```

Note: `ListQueues` is a `Query` variant but requires `project_root` which other queries don't have. Since queries already accept arbitrary fields per variant, this is fine — the query handler already receives `state` and the new variant just also needs disk access for runbook scanning. Alternatively, this could be a top-level `Request` variant instead of a `Query`. Follow whichever pattern is more consistent: if all queries are pure state reads, make `ListQueues` a `Request` variant and handle it in the main request dispatcher alongside `QueuePush`/`QueueDrop`. Use judgment based on the existing code structure — the key constraint is that it needs both `project_root` (for runbook scanning) and `state` (for item counts and worker info).

**Verify:** `cargo test -p oj-daemon` passes.

### Phase 4: Refactor CLI — rename `List` to `Items`, add new `List`

Update the CLI command definitions and handlers.

**Files:** `crates/cli/src/commands/queue.rs`

**Changes:**
1. Replace the `List` variant with two new variants:
   ```rust
   /// List all known queues
   List {
       /// Project namespace override
       #[arg(long = "project")]
       project: Option<String>,
   },
   /// Show items in a specific queue
   Items {
       /// Queue name
       queue: String,
       /// Project namespace override
       #[arg(long = "project")]
       project: Option<String>,
   },
   ```
2. Move the existing `QueueCommand::List` handler logic into `QueueCommand::Items`:
   - Same `Query::ListQueueItems` request, same display formatting
   - Change the `queue` field from `--queue` flag to positional argument
3. Add new `QueueCommand::List` handler:
   ```rust
   QueueCommand::List { project } => {
       let effective_namespace = project
           .or_else(|| std::env::var("OJ_NAMESPACE").ok())
           .unwrap_or_else(|| namespace.to_string());

       // Use the new ListQueues query (or Request variant — see Phase 3 note)
       let request = Request::Query {
           query: Query::ListQueues {
               project_root: project_root.to_path_buf(),
               namespace: effective_namespace,
           },
       };
       match client.send(&request).await? {
           Response::Queues { queues } => {
               if queues.is_empty() {
                   println!("No queues found");
                   return Ok(());
               }
               match format {
                   OutputFormat::Json => {
                       println!("{}", serde_json::to_string_pretty(&queues)?);
                   }
                   _ => {
                       for q in &queues {
                           let workers_str = if q.workers.is_empty() {
                               "-".to_string()
                           } else {
                               q.workers.join(", ")
                           };
                           println!(
                               "{}\t{}\titems={}\tworkers={}",
                               q.name, q.queue_type, q.item_count, workers_str,
                           );
                       }
                   }
               }
           }
           Response::Error { message } => anyhow::bail!("{}", message),
           _ => anyhow::bail!("unexpected response from daemon"),
       }
   }
   ```

**Verify:** `cargo build -p oj-cli` compiles. `cargo test -p oj-cli` passes. Manual smoke test:
- `oj queue list` → shows all queues with type/count/workers
- `oj queue items <name>` → shows items in that queue
- `oj queue list --queue <name>` → should error (flag no longer exists)

### Phase 5: Update docs and tests

**Files:** `docs/interface/CLI.md`, `crates/cli/src/commands/queue_tests.rs`, `crates/daemon/src/protocol_tests.rs`

**Changes:**
1. Update `CLI.md` queue section to reflect new commands:
   - `oj queue list` — list all queues
   - `oj queue items <queue>` — show items in a queue
2. Add a protocol round-trip test for `QueueSummary` / `Response::Queues` in `protocol_tests.rs`.
3. Verify existing tests still pass; update any that reference the old `--queue` flag.

**Verify:** `make check` passes (fmt, clippy, tests, build, audit, deny).

## Key Implementation Details

- **`ListQueues` needs runbook access**: Unlike `ListQueueItems` which only reads `MaterializedState`, `ListQueues` must scan `.oj/runbooks/` to discover all queue definitions (both persisted and external). This is because external queues have no WAL state — they only exist in runbook files. The runbook scan is cheap (just parsing HCL/TOML files) and is already done for every mutation request.

- **Item counts for external queues**: External queues don't store items in `MaterializedState`, so `item_count` will be 0 for them. Running the external queue's `list` command to get a count would be a side effect and potentially slow — not appropriate for a list view. The `queue_type` column makes this clear.

- **Worker discovery**: Workers are found by matching `WorkerRecord.queue_name` in `MaterializedState.workers`. Only workers that have been started (i.e., have a `WorkerRecord` in state) will appear. Workers defined in runbooks but never started won't show. This is acceptable — it shows the runtime view.

- **Namespace scoping**: Both `ListQueues` and `Items` respect the same namespace resolution: `--project` flag > `OJ_NAMESPACE` env var > resolved namespace from config.

## Verification Plan

1. **Unit tests**: Protocol round-trip test for new `QueueSummary`/`Response::Queues` types.
2. **Unit tests**: `collect_all_queues()` test in `find_tests.rs`.
3. **Integration**: `make check` — fmt, clippy, all tests, build, audit, deny.
4. **Manual smoke test**:
   - `oj queue list` with no queues defined → "No queues found"
   - `oj queue list` with queues defined → table of queues with type, count, workers
   - `oj queue items <name>` → items in that queue (same output as old `oj queue list --queue <name>`)
   - `oj queue list --queue foo` → clap error (unrecognized flag)
   - `oj queue items` (no arg) → clap error (missing required argument)
   - JSON output: `oj queue list --json`, `oj queue items <name> --json`
