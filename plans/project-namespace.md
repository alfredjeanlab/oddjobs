# Project Namespace

## Overview

Add per-project namespacing to the daemon so that workers, queues, and pipelines are scoped to the project that created them. This prevents name collisions when the single daemon serves multiple projects simultaneously.

The namespace is derived from `.oj/config.toml` (`[project].name`) with a fallback to the project root directory basename. It is stamped on events, stored in materialized state, propagated through the CLI-daemon protocol, and exposed as the `OJ_NAMESPACE` env var in shell steps and agent spawns.

## Project Structure

New/modified files:

```
crates/
├── core/
│   └── src/
│       ├── namespace.rs          # NEW: resolve_namespace() helper
│       ├── lib.rs                # export namespace module
│       └── event.rs              # add namespace field to events
├── cli/
│   └── src/
│       ├── main.rs               # resolve namespace, pass through
│       └── commands/
│           ├── pipeline.rs       # show namespace column in list
│           └── queue.rs          # --project flag, OJ_NAMESPACE fallback
├── daemon/
│   └── src/
│       ├── protocol.rs           # add namespace to Request variants and summaries
│       └── listener/
│           ├── mod.rs            # propagate namespace through dispatch
│           ├── commands.rs       # pass namespace to events
│           ├── workers.rs        # pass namespace to worker events
│           └── queues.rs         # pass namespace to queue events
├── engine/
│   └── src/
│       ├── spawn.rs              # set OJ_NAMESPACE env var
│       └── runtime/
│           ├── pipeline.rs       # set OJ_NAMESPACE in shell Effect::Shell env
│           └── handlers/
│               ├── command.rs    # set OJ_NAMESPACE in shell Effect::Shell env
│               ├── worker.rs     # namespace-aware worker state keys
│               └── lifecycle.rs  # set OJ_NAMESPACE in shell Effect::Shell env
└── storage/
    └── src/
        └── state.rs              # composite keys for workers/queues
```

## Dependencies

- `toml` crate — already in the workspace (used by `oj_runbook`). Needed for parsing `.oj/config.toml`.
- No new external dependencies required.

## Implementation Phases

### Phase 1: Namespace Resolution Helper

Add a `namespace` module to `oj_core` with a function to derive the project namespace from a `project_root` path.

**Files:**
- `crates/core/src/namespace.rs` (new)
- `crates/core/src/lib.rs` (add export)

**Details:**

```rust
// crates/core/src/namespace.rs

use std::path::Path;

/// Resolve the project namespace from a project root path.
///
/// 1. Read `.oj/config.toml` and return `[project].name` if present
/// 2. Fall back to the basename of `project_root`
/// 3. Fall back to "default" if basename is empty (e.g. root path "/")
pub fn resolve_namespace(project_root: &Path) -> String {
    if let Some(name) = read_config_name(project_root) {
        return name;
    }
    project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}

fn read_config_name(project_root: &Path) -> Option<String> {
    let config_path = project_root.join(".oj/config.toml");
    let content = std::fs::read_to_string(config_path).ok()?;
    let table: toml::Table = content.parse().ok()?;
    table
        .get("project")?
        .as_table()?
        .get("name")?
        .as_str()
        .map(String::from)
}
```

**Verification:** Unit tests for `resolve_namespace`:
- With `.oj/config.toml` containing `[project]\nname = "myproject"` → `"myproject"`
- Without config file → directory basename
- Edge case: root path → `"default"`

---

### Phase 2: Add Namespace to Events

Add an optional `namespace` field to the events listed in the spec. Use `#[serde(default)]` for backward compatibility with existing WAL entries.

**Files:**
- `crates/core/src/event.rs`

**Events to modify:**

```rust
Event::PipelineCreated {
    // ...existing fields...
    #[serde(default)]
    namespace: String,
}

Event::WorkerStarted {
    // ...existing fields...
    #[serde(default)]
    namespace: String,
}

Event::QueuePushed {
    // ...existing fields...
    #[serde(default)]
    namespace: String,
}

Event::WorkerWake {
    worker_name: String,
    #[serde(default)]
    namespace: String,
}

Event::WorkerItemDispatched {
    // ...existing fields...
    #[serde(default)]
    namespace: String,
}
```

Also update `Event::log_summary()` to include namespace in the output for these events, and update `Event::CommandRun` to carry namespace (since it's the CLI entry point that creates `PipelineCreated`).

**Verification:** Existing event serialization tests continue to pass. New tests verify round-trip serialization with namespace field. Old WAL entries without namespace deserialize with `namespace: ""`.

---

### Phase 3: Namespace in MaterializedState

Add `namespace` to `WorkerRecord` and `Pipeline`. Change the keys for `workers` and `queue_items` maps to use composite `(namespace, name)` keys.

**Files:**
- `crates/storage/src/state.rs`
- `crates/core/src/pipeline.rs`

**Changes to `Pipeline`:**

```rust
pub struct Pipeline {
    // ...existing fields...
    /// Project namespace this pipeline belongs to
    #[serde(default)]
    pub namespace: String,
}
```

Set `pipeline.namespace` in `PipelineConfig` / `Pipeline::new_with_epoch_ms`, populated from `PipelineCreated.namespace`.

**Changes to `WorkerRecord`:**

```rust
pub struct WorkerRecord {
    pub name: String,
    pub namespace: String,  // NEW
    pub project_root: PathBuf,
    // ...rest unchanged...
}
```

**Changes to `MaterializedState`:**

The `workers` map key changes from `worker_name` to `"namespace/worker_name"`. The `queue_items` map key changes from `queue_name` to `"namespace/queue_name"`.

```rust
/// Build a composite key for namespace-scoped lookups.
fn scoped_key(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        // Backward compat: old events without namespace use bare name
        name.to_string()
    } else {
        format!("{}/{}", namespace, name)
    }
}
```

Update `apply_event` for:
- `WorkerStarted`: use `scoped_key(namespace, worker_name)` as map key
- `WorkerItemDispatched`: use `scoped_key(namespace, worker_name)` for lookup
- `WorkerStopped`: needs namespace — but this event doesn't have it yet. Add `#[serde(default)] namespace: String` to `WorkerStopped` as well.
- `QueuePushed`: use `scoped_key(namespace, queue_name)` as map key
- `QueueTaken`, `QueueCompleted`, `QueueFailed`: these reference queue_name — add `#[serde(default)] namespace: String` to each for consistent keying.
- `PipelineAdvanced` terminal cleanup: iterate workers and match by scoped key.

**Important:** `PipelineCreated` handler sets `pipeline.namespace` from the event's namespace field. For old events where namespace is empty string, behavior is unchanged (bare keys).

**Verification:** State tests pass. New test: create two workers with the same name but different namespaces, verify they coexist. Snapshot deserialization backward compat test.

---

### Phase 4: Protocol and CLI Propagation

Add namespace to the daemon protocol and CLI flow.

**Files:**
- `crates/daemon/src/protocol.rs`
- `crates/cli/src/main.rs`
- `crates/cli/src/commands/queue.rs`
- `crates/daemon/src/listener/mod.rs`
- `crates/daemon/src/listener/commands.rs`
- `crates/daemon/src/listener/workers.rs`
- `crates/daemon/src/listener/queues.rs`

**Protocol changes:**

Add `namespace` to `Request` variants that carry `project_root`:

```rust
Request::RunCommand {
    project_root: PathBuf,
    invoke_dir: PathBuf,
    namespace: String,       // NEW
    command: String,
    args: Vec<String>,
    named_args: HashMap<String, String>,
}

Request::WorkerStart {
    project_root: PathBuf,
    namespace: String,       // NEW
    worker_name: String,
}

Request::WorkerWake {
    worker_name: String,
    namespace: String,       // NEW
}

Request::WorkerStop {
    worker_name: String,
    namespace: String,       // NEW
}

Request::QueuePush {
    project_root: PathBuf,
    namespace: String,       // NEW
    queue_name: String,
    data: serde_json::Value,
}
```

Add `namespace` to `PipelineSummary` and `WorkerSummary`:

```rust
pub struct PipelineSummary {
    // ...existing fields...
    #[serde(default)]
    pub namespace: String,
}

pub struct WorkerSummary {
    pub name: String,
    pub namespace: String,   // NEW
    pub queue: String,
    // ...rest unchanged...
}
```

**CLI changes in `main.rs`:**

```rust
async fn run() -> Result<()> {
    // ...existing code...
    let project_root = find_project_root();
    let namespace = oj_core::namespace::resolve_namespace(&project_root);
    // Pass namespace to command handlers that need it
    // ...
}
```

**CLI `queue.rs` changes:**

Add `--project` flag to `QueueCommand::Push`:

```rust
QueueCommand::Push {
    queue: String,
    data: Option<String>,
    #[arg(long = "var", value_parser = parse_key_value)]
    var: Vec<(String, String)>,
    /// Project namespace override
    #[arg(long = "project")]
    project: Option<String>,
}
```

Resolution order for namespace in queue push:
1. `--project` flag if provided
2. `OJ_NAMESPACE` env var if set
3. Namespace resolved from project_root (existing behavior)

**Daemon listener changes:**

Each handler receives namespace from the Request and passes it through to events. For example, in `commands.rs`, `CommandRun` event gets `namespace` from the request. The handler that creates `PipelineCreated` propagates the namespace.

**Verification:** Integration test: CLI sends request with namespace, daemon handler creates events with correct namespace. Test --project flag override.

---

### Phase 5: Environment Variable Propagation

Set `OJ_NAMESPACE` env var in shell steps, agent spawns, and tmux session creation so nested `oj` calls inherit the namespace.

**Files:**
- `crates/engine/src/spawn.rs`
- `crates/engine/src/runtime/pipeline.rs`
- `crates/engine/src/runtime/handlers/command.rs`
- `crates/engine/src/runtime/handlers/lifecycle.rs`

**Agent spawn (`spawn.rs`):**

The pipeline's `namespace` field (or the pipeline's vars) need to be available at spawn time. Add namespace to the env vars:

```rust
// In build_spawn_effects, after existing OJ_STATE_DIR logic:
// The namespace comes from the pipeline's namespace field.
// We need to thread it through — either via pipeline.namespace or as a var.

// Option: add namespace parameter to build_spawn_effects
env.push(("OJ_NAMESPACE".to_string(), namespace.to_string()));
```

The cleanest approach: `build_spawn_effects` already receives `pipeline: &Pipeline`. After Phase 3, `Pipeline` has a `namespace` field. Use `pipeline.namespace.clone()`.

```rust
// In build_spawn_effects, after OJ_DAEMON_BINARY block:
if !pipeline.namespace.is_empty() {
    env.push(("OJ_NAMESPACE".to_string(), pipeline.namespace.clone()));
}
```

**Shell steps (`pipeline.rs`, `command.rs`, `lifecycle.rs`):**

Currently, `Effect::Shell` is created with `env: HashMap::new()`. Populate it with `OJ_NAMESPACE`:

```rust
let mut env = HashMap::new();
if !pipeline.namespace.is_empty() {
    env.insert("OJ_NAMESPACE".to_string(), pipeline.namespace.clone());
}

Effect::Shell {
    pipeline_id: pipeline_id.clone(),
    step: step_name.to_string(),
    command,
    cwd: workspace_path.to_path_buf(),
    env,
}
```

This needs to happen in every place `Effect::Shell` is constructed:
- `crates/engine/src/runtime/pipeline.rs:80` — normal step execution
- `crates/engine/src/runtime/handlers/command.rs:165` — command step execution
- `crates/engine/src/runtime/handlers/lifecycle.rs:152` — shell resume

In each case, the handler has access to the pipeline (via `self.pipelines` or passed as parameter), so `pipeline.namespace` is available.

**Verification:** Unit test: spawn effects include `OJ_NAMESPACE` env var. Integration test: nested `oj queue push` inside a shell step inherits namespace.

---

### Phase 6: CLI Display

Add namespace column to `oj pipeline list` output.

**Files:**
- `crates/cli/src/commands/pipeline.rs`

**Changes:**

In the `PipelineCommand::List` text output section, add a PROJECT column:

```rust
println!(
    "{:<12} {:<15} {:<20} {:<10} {:<15} {:<10} STATUS",
    "ID", "PROJECT", "NAME", "KIND", "STEP", "UPDATED"
);
for p in &pipelines {
    let updated_ago = format_time_ago(p.updated_at_ms);
    let ns = if p.namespace.is_empty() { "-" } else { &p.namespace };
    println!(
        "{:<12} {:<15} {:<20} {:<10} {:<15} {:<10} {}",
        &p.id[..12.min(p.id.len())],
        &ns[..15.min(ns.len())],
        &p.name[..20.min(p.name.len())],
        &p.kind[..10.min(p.kind.len())],
        p.step,
        updated_ago,
        p.step_status
    );
}
```

Also update `oj pipeline show` to display namespace:

```rust
println!("  Project: {}", p.namespace);
```

And add namespace to `PipelineDetail`:

```rust
pub struct PipelineDetail {
    // ...existing fields...
    #[serde(default)]
    pub namespace: String,
}
```

**Verification:** Manual test of `oj pipeline list` showing namespace column. JSON output includes namespace field.

## Key Implementation Details

### Backward Compatibility

All new fields use `#[serde(default)]` so existing WAL entries and snapshots deserialize correctly with empty-string namespace. The `scoped_key()` function in `MaterializedState` falls back to bare names when namespace is empty, so old data continues to work.

### Namespace Inheritance Chain

```
CLI (resolve from .oj/config.toml or dirname)
  → Request (namespace field)
    → Event (namespace field)
      → MaterializedState (composite keys)
      → Pipeline.namespace
        → Effect::Shell env (OJ_NAMESPACE)
        → Effect::SpawnAgent env (OJ_NAMESPACE)
          → nested oj calls read OJ_NAMESPACE
```

### Queue Push Namespace Resolution

`oj queue push` resolves namespace in this priority order:
1. `--project` flag (explicit override)
2. `OJ_NAMESPACE` env var (inherited from parent pipeline)
3. `resolve_namespace(project_root)` (direct CLI usage)

This ensures that when a pipeline shell step runs `oj queue push`, it inherits the correct namespace from the pipeline that spawned it, without the user needing to pass `--project` explicitly.

### Worker/Queue Key Migration

The composite key format is `"namespace/name"`. For backward compat during WAL replay of old events (where namespace is ""), the key degrades to just `"name"`. This means old workers/queues keep working. New ones get namespaced keys. A future migration phase can clean up old bare keys.

### No Filtering Yet

This phase adds the namespace display column but does not filter by namespace. A follow-up phase will add `--project` filtering to `oj pipeline list`, `oj worker list`, etc.

## Verification Plan

1. **Unit tests:**
   - `namespace::resolve_namespace` with config file, without, edge cases
   - Event serialization round-trip with namespace field
   - `MaterializedState` composite key handling — two workers same name, different namespace
   - Backward compat: old events/snapshots without namespace field deserialize correctly

2. **Integration tests:**
   - CLI sends RunCommand with namespace, daemon creates pipeline with correct namespace
   - Shell step receives `OJ_NAMESPACE` env var
   - `oj queue push` inside a shell step inherits namespace from `OJ_NAMESPACE`
   - `oj queue push --project override` uses the override

3. **`make check`:**
   - `cargo fmt --all -- --check`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `quench check`
   - `cargo test --all`
   - `cargo build --all`
   - `cargo audit`
   - `cargo deny check licenses bans sources`

4. **Manual verification:**
   - `oj pipeline list` shows PROJECT column
   - `oj pipeline show <id>` shows Project field
   - Worker/queue state properly namespaced in daemon
