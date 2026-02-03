# Cron Implementation Plan

## Overview

Add cron as the third entrypoint type (alongside commands and workers). A cron runs a pipeline on a repeating timer interval. The implementation follows the worker pattern across all layers: runbook parsing, core types, engine handler, daemon protocol, and CLI.

```hcl
cron "janitor" {
  interval = "30m"
  run      = { pipeline = "cleanup" }
}
```

## Project Structure

New and modified files, organized by crate:

```
crates/runbook/src/
├── cron.rs                    # NEW: CronDef struct
├── lib.rs                     # ADD: mod cron, pub use CronDef
├── parser.rs                  # ADD: crons field to Runbook, name fixup, validation
├── find.rs                    # ADD: find_runbook_by_cron(), collect helper

crates/core/src/
├── event.rs                   # ADD: CronStarted, CronStopped, CronFired events
├── timer.rs                   # ADD: TimerId::cron() constructor + is_cron()

crates/storage/src/
├── state.rs                   # ADD: CronRecord, crons HashMap, apply_event arms

crates/engine/src/
├── runtime/handlers/
│   ├── mod.rs                 # ADD: mod cron, event routing
│   └── cron.rs                # NEW: cron handler (start/stop/fire)
├── runtime/handlers/timer.rs  # ADD: cron timer routing

crates/daemon/src/
├── protocol.rs                # ADD: CronStart/Stop/Once requests, CronStarted/Crons responses, CronSummary, ListCrons query
├── listener/
│   ├── mod.rs                 # ADD: mod crons, request routing
│   └── crons.rs               # NEW: handle_cron_start/stop/once
├── lifecycle.rs               # ADD: resume running crons on daemon restart

crates/cli/src/
├── commands/
│   ├── mod.rs                 # ADD: pub mod cron
│   └── cron.rs                # NEW: CronArgs, CronCommand, handle()
├── main.rs                    # ADD: Cron variant to Commands enum, dispatch
```

## Dependencies

No new external crates needed. The existing `parse_duration` function in `crates/engine/src/monitor.rs` handles duration strings like "30m", "6h", "24h". The `validate_duration_str` function in `crates/runbook/src/validate.rs` validates the format at parse time.

## Implementation Phases

### Phase 1: Runbook Parser — CronDef and Parsing

Add the cron definition type and integrate it into the runbook parser.

**New file: `crates/runbook/src/cron.rs`**

```rust
use crate::RunDirective;
use serde::{Deserialize, Serialize};

/// A cron definition that runs a pipeline on a timer interval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronDef {
    /// Cron name (injected from map key)
    #[serde(skip)]
    pub name: String,
    /// Interval duration string (e.g. "30m", "6h", "24h")
    pub interval: String,
    /// What to run (pipeline reference only)
    pub run: RunDirective,
}
```

**Modifications:**

1. **`crates/runbook/src/lib.rs`** — Add `mod cron;` and `pub use cron::CronDef;`

2. **`crates/runbook/src/parser.rs`** — Add to `Runbook` struct:
   ```rust
   #[serde(default, alias = "cron")]
   pub crons: HashMap<String, CronDef>,
   ```
   Add name fixup in `parse_runbook_with_format`:
   ```rust
   for (name, cron) in &mut runbook.crons {
       cron.name = name.clone();
   }
   ```
   Add validation (after worker validation, new section ~step 6.5):
   - Validate `interval` with `validate_duration_str`
   - Validate `run` is a pipeline reference (not shell or agent): `cron.run.pipeline_name().is_some()`
   - Validate the referenced pipeline exists in the runbook
   ```rust
   for (name, cron) in &runbook.crons {
       // Validate interval
       if let Err(e) = validate_duration_str(&cron.interval) {
           return Err(ParseError::InvalidFormat {
               location: format!("cron.{}.interval", name),
               message: e,
           });
       }
       // Validate run is a pipeline reference
       let pipeline_name = match cron.run.pipeline_name() {
           Some(p) => p,
           None => {
               return Err(ParseError::InvalidFormat {
                   location: format!("cron.{}.run", name),
                   message: "cron run must reference a pipeline".to_string(),
               });
           }
       };
       if !runbook.pipelines.contains_key(pipeline_name) {
           return Err(ParseError::InvalidFormat {
               location: format!("cron.{}.run", name),
               message: format!(
                   "references unknown pipeline '{}'; available pipelines: {}",
                   pipeline_name,
                   sorted_keys(&runbook.pipelines),
               ),
           });
       }
   }
   ```

3. **`crates/runbook/src/parser.rs`** — Add `get_cron` accessor to `Runbook` impl:
   ```rust
   pub fn get_cron(&self, name: &str) -> Option<&CronDef> {
       self.crons.get(name)
   }
   ```

4. **`crates/runbook/src/find.rs`** — Add `find_runbook_by_cron`:
   ```rust
   pub fn find_runbook_by_cron(
       runbook_dir: &Path,
       name: &str,
   ) -> Result<Option<Runbook>, FindError> {
       find_runbook(runbook_dir, name, |rb| rb.get_cron(name).is_some())
   }
   ```
   Export from `crates/runbook/src/lib.rs`.

**Tests:**
- Unit test in `crates/runbook/src/parser_tests/` (or add to existing mod): parse a valid cron HCL block, verify fields
- Test invalid interval rejected
- Test non-pipeline run directive rejected
- Test unknown pipeline reference rejected

**Milestone:** `cargo test -p oj-runbook` passes with cron parsing tests.

---

### Phase 2: Core Types — Events, TimerId, State

Add cron events, timer ID variant, and state record.

**`crates/core/src/event.rs`** — Add cron events (after worker events):

```rust
// -- cron --
#[serde(rename = "cron:started")]
CronStarted {
    cron_name: String,
    project_root: PathBuf,
    runbook_hash: String,
    interval: String,
    pipeline_name: String,
    #[serde(default)]
    namespace: String,
},

#[serde(rename = "cron:stopped")]
CronStopped {
    cron_name: String,
    #[serde(default)]
    namespace: String,
},

#[serde(rename = "cron:fired")]
CronFired {
    cron_name: String,
    pipeline_id: PipelineId,
    #[serde(default)]
    namespace: String,
},
```

Also add to `Event::name()`, `Event::log_summary()`, and the no-op arm in `Event::pipeline_id()` (CronFired returns `Some`).

**`crates/core/src/timer.rs`** — Add cron timer constructor:

```rust
/// Timer ID for a cron interval tick.
pub fn cron(cron_name: &str, namespace: &str) -> Self {
    if namespace.is_empty() {
        Self::new(format!("cron:{}", cron_name))
    } else {
        Self::new(format!("cron:{}/{}", namespace, cron_name))
    }
}

pub fn is_cron(&self) -> bool {
    self.0.starts_with("cron:")
}
```

**`crates/storage/src/state.rs`** — Add cron record and state:

```rust
/// Record of a running cron for WAL replay / restart recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRecord {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    pub project_root: PathBuf,
    pub runbook_hash: String,
    /// "running" or "stopped"
    pub status: String,
    pub interval: String,
    pub pipeline_name: String,
}
```

Add to `MaterializedState`:
```rust
#[serde(default)]
pub crons: HashMap<String, CronRecord>,
```

Add `apply_event` arms:
- `CronStarted` → insert/update `CronRecord` with status "running"
- `CronStopped` → set status to "stopped"
- `CronFired` → no-op for state (pipeline creation is handled by PipelineCreated)

**Tests:**
- Unit tests for `MaterializedState::apply_event` with cron events
- Timer ID construction and `is_cron()` tests

**Milestone:** `cargo test -p oj-core -p oj-storage` passes.

---

### Phase 3: Engine — Cron Handler and Timer Routing

Implement the cron runtime handler that manages interval timers and pipeline spawning.

**New file: `crates/engine/src/runtime/handlers/cron.rs`**

In-memory state:
```rust
pub(crate) struct CronState {
    pub project_root: PathBuf,
    pub runbook_hash: String,
    pub interval: String,
    pub pipeline_name: String,
    pub status: CronStatus,
    pub namespace: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CronStatus {
    Running,
    Stopped,
}
```

Store `cron_states: Arc<Mutex<HashMap<String, CronState>>>` on Runtime (follow `worker_states` pattern).

Handlers:

1. **`handle_cron_started`** — Parse interval via `parse_duration`, store CronState, set first interval timer via `Effect::SetTimer { id: TimerId::cron(&cron_name, &namespace), duration }`.

2. **`handle_cron_stopped`** — Update CronState status to Stopped, cancel timer via `Effect::CancelTimer { id: TimerId::cron(&cron_name, &namespace) }`.

3. **`handle_cron_timer_fired`** — Called when the cron timer fires. Spawns the pipeline (via `create_pipeline` / the same `CreatePipelineParams` path used by workers), then re-sets the timer for the next interval. The pipeline is created with `kind` = cron's pipeline_name and `name` formatted as `"cron/{cron_name}/{short_id}"`.

**`crates/engine/src/runtime/handlers/timer.rs`** — Add cron routing:

```rust
if let Some(rest) = id_str.strip_prefix("cron:") {
    return self.handle_cron_timer_fired(rest).await;
}
```

**`crates/engine/src/runtime/handlers/mod.rs`** — Add:
- `pub(crate) mod cron;`
- Event routing in `handle_event`:
  ```rust
  Event::CronStarted { cron_name, project_root, runbook_hash, namespace, .. } => {
      result_events.extend(
          self.handle_cron_started(cron_name, project_root, runbook_hash, namespace).await?
      );
  }
  Event::CronStopped { cron_name, namespace, .. } => {
      result_events.extend(self.handle_cron_stopped(cron_name, namespace).await?);
  }
  ```
- Add `CronFired` to the no-op section (pipeline creation is already emitted inline by the handler)

**`crates/engine/src/runtime/mod.rs`** — Add `cron_states` field to `Runtime` struct, initialize in constructor.

**Pipeline creation detail:** When the cron timer fires, the handler:
1. Reads the cached runbook to get the pipeline definition
2. Calls `create_pipeline` with `kind = pipeline_name`, `name = pipeline_display_name(pipeline_name)`, `vars = {}` (empty — crons have no input), `cwd = project_root`
3. Emits `CronFired { cron_name, pipeline_id, namespace }` for logging/tracking
4. Re-sets the timer: `Effect::SetTimer { id: TimerId::cron(&cron_name, &namespace), duration }`

**Tests** (in `crates/engine/src/runtime_tests/cron.rs`):
- `cron_started_sets_timer`: Process CronStarted, verify SetTimer effect
- `cron_timer_fires_creates_pipeline_and_resets_timer`: Advance FakeClock past interval, verify PipelineCreated event emitted and timer re-set
- `cron_stopped_cancels_timer`: Process CronStopped, verify CancelTimer effect
- `cron_restart_resumes_timer`: Simulate WAL replay with CronStarted, verify timer set

**Milestone:** `cargo test -p oj-engine` passes with cron handler tests.

---

### Phase 4: Daemon — Protocol, Listener, Lifecycle

Add IPC protocol messages and wire up cron requests in the daemon.

**`crates/daemon/src/protocol.rs`** — Add to `Request`:
```rust
/// Start a cron timer
CronStart {
    project_root: PathBuf,
    #[serde(default)]
    namespace: String,
    cron_name: String,
},

/// Stop a cron timer
CronStop {
    cron_name: String,
    #[serde(default)]
    namespace: String,
},

/// Run the cron's pipeline once immediately (no timer)
CronOnce {
    project_root: PathBuf,
    #[serde(default)]
    namespace: String,
    cron_name: String,
},
```

Add to `Query`:
```rust
/// List all crons and their status
ListCrons,
```

Add to `Response`:
```rust
/// Cron started successfully
CronStarted { cron_name: String },

/// List of crons
Crons { crons: Vec<CronSummary> },
```

Add summary type:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronSummary {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    pub interval: String,
    pub pipeline: String,
    pub status: String,
}
```

**New file: `crates/daemon/src/listener/crons.rs`** — Follow `workers.rs` pattern:

1. **`handle_cron_start`** — Load runbook, validate cron exists, validate pipeline exists, hash runbook, emit `RunbookLoaded` + `CronStarted` events. Return `Response::CronStarted`.

2. **`handle_cron_stop`** — Emit `CronStopped` event. Return `Response::Ok`.

3. **`handle_cron_once`** — Load runbook, validate cron exists, hash runbook, emit `RunbookLoaded`, then emit `CommandRun`-style pipeline creation (using the same path as `commands.rs` but with `kind = "cron"` pipeline name). Return `Response::CommandStarted { pipeline_id, pipeline_name }`.

**`crates/daemon/src/listener/mod.rs`** — Add `mod crons;` and routing in `handle_request`:
```rust
Request::CronStart { project_root, namespace, cron_name } =>
    crons::handle_cron_start(&project_root, &namespace, &cron_name, event_bus),

Request::CronStop { cron_name, namespace } =>
    crons::handle_cron_stop(&cron_name, &namespace, event_bus),

Request::CronOnce { project_root, namespace, cron_name } =>
    crons::handle_cron_once(&project_root, &namespace, &cron_name, event_bus, state).await,
```

Add `ListCrons` query handling in `query.rs`:
```rust
Query::ListCrons => {
    let crons = state.crons.values().map(|c| CronSummary {
        name: c.name.clone(),
        namespace: c.namespace.clone(),
        interval: c.interval.clone(),
        pipeline: c.pipeline_name.clone(),
        status: c.status.clone(),
    }).collect();
    Response::Crons { crons }
}
```

**`crates/daemon/src/lifecycle.rs`** — Add cron auto-resume after the worker resume block:
```rust
// Resume crons that were running before the daemon restarted.
let running_crons: Vec<_> = state
    .crons
    .values()
    .filter(|c| c.status == "running")
    .collect();

if !running_crons.is_empty() {
    info!("Resuming {} running crons", running_crons.len());
}

for cron in &running_crons {
    info!(
        cron = %cron.name,
        namespace = %cron.namespace,
        "resuming cron after daemon restart"
    );
    let _ = event_tx
        .send(Event::CronStarted {
            cron_name: cron.name.clone(),
            project_root: cron.project_root.clone(),
            runbook_hash: cron.runbook_hash.clone(),
            interval: cron.interval.clone(),
            pipeline_name: cron.pipeline_name.clone(),
            namespace: cron.namespace.clone(),
        })
        .await;
}
```

**Tests:**
- `crates/daemon/src/listener/crons_tests.rs`: test cron start/stop/once request handling
- `crates/daemon/src/lifecycle_tests.rs`: test `reconcile_state_resumes_running_crons`

**Milestone:** `cargo test -p oj-daemon` passes with cron tests.

---

### Phase 5: CLI — `oj cron` Subcommand

Add the CLI commands for cron management.

**New file: `crates/cli/src/commands/cron.rs`** — Follow `worker.rs` pattern:

```rust
use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::output::OutputFormat;

use oj_daemon::{Query, Request, Response};

#[derive(Args)]
pub struct CronArgs {
    #[command(subcommand)]
    pub command: CronCommand,
}

#[derive(Subcommand)]
pub enum CronCommand {
    /// List all crons and their status
    List {},
    /// Start a cron (begins interval timer)
    Start {
        /// Cron name from runbook
        name: String,
    },
    /// Stop a cron (cancels interval timer)
    Stop {
        /// Cron name from runbook
        name: String,
    },
    /// Run the cron's pipeline once now (ignores interval)
    Once {
        /// Cron name from runbook
        name: String,
    },
}
```

Handler dispatches to daemon via `Request::CronStart`, `Request::CronStop`, `Request::CronOnce`, and `Query::ListCrons`.

List output format (text mode):
```
NAME      INTERVAL  PIPELINE  STATUS
janitor   30m       cleanup   running
nightly   24h       deploy    stopped
```

**`crates/cli/src/commands/mod.rs`** — Add `pub mod cron;`

**`crates/cli/src/main.rs`** — Add to `Commands` enum:
```rust
/// Cron management
Cron(cron::CronArgs),
```

Add dispatch (follow worker pattern — List is query, rest are actions):
```rust
Commands::Cron(args) => match &args.command {
    cron::CronCommand::List { .. } => {
        let client = DaemonClient::for_query()?;
        cron::handle(args.command, &client, &project_root, &namespace, format).await?
    }
    _ => {
        let client = DaemonClient::for_action()?;
        cron::handle(args.command, &client, &project_root, &namespace, format).await?
    }
},
```

**Milestone:** `cargo build -p oj-cli` succeeds; `oj cron --help` works.

---

### Phase 6: Integration and Cleanup

1. Add crons to the `StatusOverview` response in `protocol_status.rs` (count running crons per namespace)
2. Update `crates/daemon/src/listener/query.rs` `StatusOverview` handler to include cron counts
3. Run `make check` (fmt, clippy, tests, build, audit, deny)
4. Verify end-to-end: write a test runbook with a cron, start it, observe pipeline creation

**Milestone:** `make check` passes clean.

## Key Implementation Details

### Timer Lifecycle

Cron timers are one-shot timers that self-reschedule. The pattern:

1. `CronStarted` → `Effect::SetTimer { id: TimerId::cron(name, ns), duration }`
2. Timer fires → `Event::TimerStart { id }` → handler creates pipeline + `Effect::SetTimer` (reschedule)
3. `CronStopped` → `Effect::CancelTimer { id: TimerId::cron(name, ns) }`

This matches the existing timer model where timers are removed after firing (see `Scheduler::fired_timers`), so re-setting is required after each fire.

### Duration Parsing

Reuse `oj_engine::monitor::parse_duration` in the engine handler to convert the interval string to `std::time::Duration`. The runbook parser validates the format at parse time with `validate_duration_str`.

### Pipeline Creation from Cron

When the cron fires, create a pipeline using the same `CreatePipelineParams` infrastructure used by commands and workers. The pipeline `kind` is the pipeline definition name, and the display `name` is generated via `pipeline_display_name`. Variables (`vars`) are empty since crons have no input arguments.

### Idempotent Start

Following the worker pattern, `CronStart` is idempotent. Re-starting an already-running cron overwrites the in-memory CronState and resets the timer. This means `oj cron start <name>` also serves to update the interval if the runbook changed.

### Namespace Scoping

Cron state keys use `scoped_key(namespace, cron_name)` for the `MaterializedState.crons` HashMap, consistent with workers and queues.

### CronOnce Implementation

`oj cron once` bypasses the timer system entirely. The daemon listener loads the runbook, resolves the pipeline, and directly emits a `CommandRun` event (or equivalent pipeline creation) to spawn the pipeline immediately. This allows testing the cron's pipeline without waiting for the interval.

## Verification Plan

### Unit Tests
- **Runbook parsing**: Valid cron HCL parses correctly; invalid interval, non-pipeline run, and missing pipeline reference all produce errors
- **Core events**: CronStarted/CronStopped/CronFired serialize/deserialize correctly
- **State materialization**: apply_event for cron events updates CronRecord correctly
- **Timer ID**: `TimerId::cron()` produces expected format, `is_cron()` identifies them

### Engine Tests (with FakeClock)
- CronStarted sets timer with correct duration
- Advancing FakeClock past interval fires timer and creates pipeline
- Timer is re-set after firing (interval repeats)
- CronStopped cancels the timer
- Daemon restart replays CronStarted and re-establishes timer

### Daemon Tests
- Listener correctly routes CronStart/Stop/Once requests
- CronStart emits RunbookLoaded + CronStarted events
- CronStop emits CronStopped event
- CronOnce creates pipeline without setting timer
- reconcile_state resumes running crons

### CLI Tests
- Build succeeds, `--help` output includes cron subcommands
- List formats correctly (text and JSON modes)

### Integration (manual or e2e)
- Create a runbook with a cron and short interval (e.g. "5s")
- `oj cron start janitor` → cron starts
- `oj cron list` → shows running cron
- Observe pipeline created after interval
- `oj cron stop janitor` → cron stops, no more pipelines
- `oj cron once janitor` → single pipeline created immediately
- Restart daemon → cron auto-resumes
