# Plan: Add `--project` flag to all list commands

## Overview

Add `--project` flag support to the six list/prune commands that currently show cross-project data without filtering: `pipeline list`, `agent list`, `session list`, `cron list`, `cron prune`, and `workspace list`. All summary structs already have a `namespace` field, so this is purely a CLI-side change using client-side filtering — no daemon protocol changes needed.

## Project Structure

Files to modify:

```
crates/cli/src/commands/pipeline.rs   # Add --project to List variant + filtering
crates/cli/src/commands/agent.rs      # Add --project to List variant + filtering
crates/cli/src/commands/session.rs    # Add --project to List variant + filtering
crates/cli/src/commands/cron.rs       # Add --project to List and Prune variants + filtering
crates/cli/src/commands/workspace.rs  # Add --project to List variant + filtering
```

No new files. No changes to `main.rs` or daemon code.

## Dependencies

None. Uses existing `clap` `#[arg]` attributes and `std::env::var`.

## Implementation Phases

### Phase 1: `pipeline list` — Add `--project` flag with client-side filtering

**File:** `crates/cli/src/commands/pipeline.rs`

1. Add `project` field to `PipelineCommand::List` (lines 25-40):

```rust
List {
    /// Filter by name substring
    name: Option<String>,

    /// Filter by status (e.g. "running", "failed", "completed")
    #[arg(long)]
    status: Option<String>,

    /// Maximum number of pipelines to show (default: 20)
    #[arg(short = 'n', long, default_value = "20")]
    limit: usize,

    /// Show all pipelines (no limit)
    #[arg(long, conflicts_with = "limit")]
    no_limit: bool,

    /// Filter by project namespace
    #[arg(long = "project")]
    project: Option<String>,
},
```

2. In the `handle` match arm for `List` (around line 308), destructure the new `project` field and add filtering before existing name/status filters:

```rust
PipelineCommand::List {
    name,
    status,
    limit,
    no_limit,
    project,
} => {
    let mut pipelines = client.list_pipelines().await?;

    // Filter by project namespace
    let filter_namespace = project.or_else(|| std::env::var("OJ_NAMESPACE").ok());
    if let Some(ref ns) = filter_namespace {
        pipelines.retain(|p| p.namespace == *ns);
    }

    // Filter by name substring (existing)
    // ...
```

**Verify:** `cargo check -p oj-cli`

### Phase 2: `agent list` — Add `--project` flag with client-side filtering

**File:** `crates/cli/src/commands/agent.rs`

1. Add `project` field to `AgentCommand::List` (lines 31-47):

```rust
List {
    /// Filter by pipeline ID (or prefix)
    #[arg(long)]
    pipeline: Option<String>,

    /// Filter by status (e.g. "running", "completed", "failed", "waiting")
    #[arg(long)]
    status: Option<String>,

    /// Maximum number of agents to show (default: 20)
    #[arg(short = 'n', long, default_value = "20")]
    limit: usize,

    /// Show all agents (no limit)
    #[arg(long, conflicts_with = "limit")]
    no_limit: bool,

    /// Filter by project namespace
    #[arg(long = "project")]
    project: Option<String>,
},
```

2. In the `handle` match arm for `List` (around line 139), destructure `project` and add filtering after fetching agents. Note: `AgentSummary.namespace` is `Option<String>`, so the comparison differs slightly:

```rust
AgentCommand::List {
    pipeline,
    status,
    limit,
    no_limit,
    project,
} => {
    let mut agents = client
        .list_agents(pipeline.as_deref(), status.as_deref())
        .await?;

    // Filter by project namespace
    let filter_namespace = project.or_else(|| std::env::var("OJ_NAMESPACE").ok());
    if let Some(ref ns) = filter_namespace {
        agents.retain(|a| a.namespace.as_deref() == Some(ns.as_str()));
    }

    let total = agents.len();
    // ... rest unchanged
```

The `agents` binding must change from `let` to `let mut`.

**Verify:** `cargo check -p oj-cli`

### Phase 3: `session list` and `cron list`/`cron prune` — Add `--project` flag

**File:** `crates/cli/src/commands/session.rs`

1. Add `project` field to `SessionCommand::List`:

```rust
/// List all sessions
List {
    /// Filter by project namespace
    #[arg(long = "project")]
    project: Option<String>,
},
```

2. In the `handle` match arm (line 62), destructure `project` and add filtering:

```rust
SessionCommand::List { project } => {
    let mut sessions = client.list_sessions().await?;

    // Filter by project namespace
    let filter_namespace = project.or_else(|| std::env::var("OJ_NAMESPACE").ok());
    if let Some(ref ns) = filter_namespace {
        sessions.retain(|s| s.namespace == *ns);
    }

    match format {
    // ... rest unchanged
```

The `sessions` binding must change from `let` to `let mut`.

**File:** `crates/cli/src/commands/cron.rs`

3. Add `project` field to `CronCommand::List`:

```rust
/// List all crons and their status
List {
    /// Filter by project namespace
    #[arg(long = "project")]
    project: Option<String>,
},
```

4. In the `handle` match arm for `List` (line 200), destructure `project` and add filtering after fetching crons:

```rust
CronCommand::List { project } => {
    let request = Request::Query {
        query: Query::ListCrons,
    };
    match client.send(&request).await? {
        Response::Crons { mut crons } => {
            // Filter by project namespace
            let filter_namespace =
                project.or_else(|| std::env::var("OJ_NAMESPACE").ok());
            if let Some(ref ns) = filter_namespace {
                crons.retain(|c| c.namespace == *ns);
            }

            crons.sort_by(|a, b| a.name.cmp(&b.name));
            // ... rest unchanged
```

5. Add `project` field to `CronCommand::Prune`:

```rust
Prune {
    /// Prune all stopped crons (currently same as default)
    #[arg(long)]
    all: bool,

    /// Show what would be pruned without making changes
    #[arg(long)]
    dry_run: bool,

    /// Filter by project namespace
    #[arg(long = "project")]
    project: Option<String>,
},
```

6. In the `handle` match arm for `Prune` (line 168), destructure `project` and filter the pruned results. The `cron_prune` function returns `(pruned, skipped)` — filter `pruned` by namespace if `--project` is specified:

```rust
CronCommand::Prune {
    all,
    dry_run,
    project,
} => {
    let (mut pruned, skipped) = client.cron_prune(all, dry_run).await?;

    // Filter by project namespace
    let filter_namespace = project.or_else(|| std::env::var("OJ_NAMESPACE").ok());
    if let Some(ref ns) = filter_namespace {
        pruned.retain(|e| e.namespace == *ns);
    }
    // ... rest unchanged (use pruned.len() for counts)
```

Note: For `cron prune`, the daemon has already performed the prune. The `--project` flag filters the _display_ of what was pruned — this is consistent with how `pipeline prune --project` works (it passes the project to the daemon for server-side filtering). However, `cron_prune` doesn't currently accept a namespace parameter. There are two options:

- **(A)** Filter display only (simpler, matches the scope of this plan)
- **(B)** Thread namespace through `cron_prune` to the daemon (requires protocol change)

Use option **(A)** for now — filter the display. If scoped pruning is needed, it can be added later.

**Verify:** `cargo check -p oj-cli`

### Phase 4: `workspace list` — Add `--project` flag

**File:** `crates/cli/src/commands/workspace.rs`

1. Add `project` field to `WorkspaceCommand::List`:

```rust
List {
    /// Maximum number of workspaces to show (default: 20)
    #[arg(short = 'n', long, default_value = "20")]
    limit: usize,

    /// Show all workspaces (no limit)
    #[arg(long, conflicts_with = "limit")]
    no_limit: bool,

    /// Filter by project namespace
    #[arg(long = "project")]
    project: Option<String>,
},
```

2. In the `handle` match arm (line 68), destructure `project` and add filtering before sorting/limiting:

```rust
WorkspaceCommand::List {
    limit,
    no_limit,
    project,
} => {
    let mut workspaces = client.list_workspaces().await?;

    // Filter by project namespace
    let filter_namespace = project.or_else(|| std::env::var("OJ_NAMESPACE").ok());
    if let Some(ref ns) = filter_namespace {
        workspaces.retain(|w| w.namespace == *ns);
    }

    // Sort by recency (most recent first)
    workspaces.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    // ... rest unchanged
```

**Verify:** `cargo check -p oj-cli`

### Phase 5: Final verification

Run `make check` which covers:
- `cargo fmt --all`
- `cargo clippy --all -- -D warnings`
- `cargo build --all`
- `cargo test --all`

## Key Implementation Details

1. **Client-side filtering pattern** — All changes follow the existing pattern from `worker list` (`crates/cli/src/commands/worker.rs:143-155`): fetch all data from the daemon, then `retain()` by namespace on the client. This avoids any daemon protocol changes.

2. **Namespace resolution priority** — The `--project` flag takes precedence over the `OJ_NAMESPACE` environment variable. If neither is set, no filtering is applied (all entities shown). This matches the worker list behavior:
   ```rust
   let filter_namespace = project.or_else(|| std::env::var("OJ_NAMESPACE").ok());
   ```

3. **No function signature changes** — The `project` field comes from the clap enum variant, and `OJ_NAMESPACE` is read from the environment. No `handle` function signatures need to change.

4. **AgentSummary namespace is `Option<String>`** — Unlike other summary types where `namespace` is `String`, `AgentSummary.namespace` is `Option<String>`. The filtering comparison must use `.as_deref()`: `a.namespace.as_deref() == Some(ns.as_str())`.

5. **Cron prune display-only filtering** — The `cron prune` command's `--project` flag filters the displayed output rather than scoping the prune operation server-side. This is a pragmatic choice to avoid protocol changes. The skipped count from the daemon won't reflect the project filter — this is acceptable for now.

## Verification Plan

1. **Build:** `cargo check -p oj-cli` after each phase
2. **Lint:** `cargo clippy --all -- -D warnings`
3. **Tests:** `cargo test --all` (existing tests should pass unchanged)
4. **Full CI:** `make check`
5. **Manual testing:**
   - `oj pipeline list --project myproject` — shows only pipelines from `myproject`
   - `oj agent list --project myproject` — shows only agents from `myproject`
   - `oj session list --project myproject` — shows only sessions from `myproject`
   - `oj cron list --project myproject` — shows only crons from `myproject`
   - `oj workspace list --project myproject` — shows only workspaces from `myproject`
   - `oj cron prune --project myproject` — only displays pruned crons from `myproject`
   - Each command without `--project` — shows all entities (no regression)
   - With `OJ_NAMESPACE=myproject oj pipeline list` — filters without explicit flag
