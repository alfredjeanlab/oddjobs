# Plan: `oj project list`

## Overview

Add an `oj project list` command that shows all projects with active work in the daemon. For each project, display the project name (namespace) and root directory, derived from active workers, pipelines, agents, and crons. This is a read-only query command — no daemon auto-start.

## Project Structure

Files to create or modify:

```
crates/cli/src/commands/project.rs     # NEW — CLI command module
crates/cli/src/commands/mod.rs         # ADD pub mod project
crates/cli/src/main.rs                 # ADD Commands::Project variant + dispatch
crates/daemon/src/protocol.rs          # ADD Query::ListProjects, Response::Projects, ProjectSummary
crates/daemon/src/listener/query.rs    # ADD ListProjects query handler
crates/cli/src/client.rs              # ADD list_projects() helper
```

## Dependencies

No new external dependencies. Uses existing `clap`, `serde`, `serde_json`.

## Implementation Phases

### Phase 1: Protocol — Add `ListProjects` query and response types

**Files:** `crates/daemon/src/protocol.rs`

1. Add `Query::ListProjects` variant (no parameters — lists all projects globally):

```rust
// In enum Query:
/// List all projects with active work
ListProjects,
```

2. Add `ProjectSummary` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectSummary {
    pub name: String,
    pub root: PathBuf,
    pub active_pipelines: usize,
    pub active_agents: usize,
    pub workers: usize,
    pub crons: usize,
}
```

3. Add `Response::Projects` variant:

```rust
// In enum Response:
/// List of projects with active work
Projects { projects: Vec<ProjectSummary> },
```

**Verify:** `cargo check -p oj-daemon`

### Phase 2: Daemon — Implement `ListProjects` query handler

**File:** `crates/daemon/src/listener/query.rs`

Add a match arm for `Query::ListProjects` that:

1. Iterates `state.workers` — each `WorkerRecord` has `.namespace` and `.project_root`. Only count workers with `status == "running"`.
2. Iterates `state.crons` — each `CronRecord` has `.namespace` and `.project_root`. Only count crons with `status == "running"`.
3. Iterates `state.pipelines` — each `Pipeline` has `.namespace`. Count non-terminal pipelines. Extract active agents from step history (same logic as `StatusOverview`).
4. Build a `BTreeMap<String, ProjectSummary>` keyed by namespace. For `project_root`, prefer the value from workers/crons (which store it directly). Pipelines only have namespace, not project_root — so only workers and crons contribute root paths.
5. Only include projects that have at least one active entity (running worker, running cron, non-terminal pipeline, or active agent).
6. Return `Response::Projects { projects }` sorted by name.

Key pattern — deriving `project_root` per namespace:

```rust
// Workers and crons store project_root; use the first one found per namespace
let mut ns_roots: BTreeMap<String, PathBuf> = BTreeMap::new();
for w in state.workers.values() {
    if w.status == "running" {
        ns_roots.entry(w.namespace.clone()).or_insert_with(|| w.project_root.clone());
    }
}
for c in state.crons.values() {
    if c.status == "running" {
        ns_roots.entry(c.namespace.clone()).or_insert_with(|| c.project_root.clone());
    }
}
```

For pipelines that have no corresponding worker/cron, the root directory won't be known — display as empty/unknown. This is acceptable because pipelines without workers or crons are ad-hoc runs where the project root wasn't persisted in daemon state.

**Verify:** `cargo check -p oj-daemon`

### Phase 3: CLI client helper

**File:** `crates/cli/src/client.rs`

Add a method following the existing pattern (e.g. `status_overview()`):

```rust
pub async fn list_projects(&self) -> Result<Vec<oj_daemon::ProjectSummary>, ClientError> {
    let request = Request::Query { query: Query::ListProjects };
    match self.send(&request).await? {
        Response::Projects { projects } => Ok(projects),
        other => Self::reject(other),
    }
}
```

**Verify:** `cargo check -p oj-cli`

### Phase 4: CLI command module and wiring

**Files:** `crates/cli/src/commands/project.rs` (new), `commands/mod.rs`, `main.rs`

1. Create `project.rs` with:
   - `ProjectArgs` struct with `#[command(subcommand)]` field
   - `ProjectCommand` enum with `List` variant
   - `handle()` function that calls `client.list_projects()` and formats output

2. Text output format — a simple table:

```
NAME            ROOT
oddjobs         /home/user/oddjobs
my-app          /home/user/my-app
```

When there's activity detail, show counts inline:

```
NAME            ROOT                          PIPELINES  WORKERS  AGENTS  CRONS
oddjobs         /home/user/oddjobs            2          1        2       0
my-app          /home/user/my-app             0          1        0       1
```

If root is unknown (no worker/cron, only pipelines), show `(unknown)`.

3. JSON output: serialize the `Vec<ProjectSummary>` directly.

4. Wire up in `mod.rs`:
```rust
pub mod project;
```

5. Wire up in `main.rs`:
```rust
// In Commands enum:
/// Project management
Project(project::ProjectArgs),

// In run() dispatch — query command:
Commands::Project(args) => {
    let client = DaemonClient::for_query()?;
    project::handle(args.command, &client, format).await?
}
```

Note: `oj project list` does NOT need `project_root` or `namespace` from the local directory — it lists all projects globally. So it goes before the `find_project_root()` call or uses `for_query()` without namespace context.

**Verify:** `cargo check -p oj-cli`, then `make check`

## Key Implementation Details

1. **No new IPC round-trips needed** — a single `ListProjects` query returns everything. The daemon already has all the data in `MaterializedState`.

2. **project_root sourcing** — Workers and crons persist `project_root` in their records. Pipelines do not. The implementation should prefer worker/cron roots. For namespaces that only have pipelines (no running workers/crons), also check stopped workers/crons as a fallback (they retain `project_root` until pruned).

3. **"Active" definition** — A project is active if it has any: running workers (`status == "running"`), running crons (`status == "running"`), non-terminal pipelines, or active agents. Stopped workers/crons alone do not make a project "active" — but their `project_root` can still be used if the namespace has active pipelines.

4. **Global command** — Unlike most commands that are namespace-scoped, `oj project list` is global (cross-project). It follows the same pattern as `oj status` — using `DaemonClient::for_query()` without namespace filtering.

5. **Graceful when daemon is down** — Follow the `oj status` pattern: if the daemon isn't running, print "oj daemon: not running" (text) or `{"status": "not_running"}` (JSON) and exit 0.

## Verification Plan

1. **Unit:** No new state machine logic, so no unit tests needed.
2. **Build:** `cargo check --all`, `cargo clippy --all-targets --all-features -- -D warnings`
3. **Manual testing:**
   - `oj project list` with daemon stopped → shows "not running"
   - `oj project list` with daemon running, no active work → empty list
   - Start a worker in one project, start a cron in another → both appear
   - `oj project list -o json` → valid JSON output
4. **`make check`** — full CI-equivalent verification
