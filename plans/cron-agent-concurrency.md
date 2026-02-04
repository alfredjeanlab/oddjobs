# Cron Agent Support & Agent Max Concurrency

## Overview

Two features that work together to make crons more flexible and agents safer to run on timers:

1. **Cron → Agent**: Allow cron blocks to use `run = { agent = "name" }` in addition to `run = { pipeline = "name" }`. The agent is spawned as a standalone agent (same as a command with `run = { agent }`).

2. **Agent max_concurrency**: Add a `max_concurrency` field to agent definitions. When set, the engine checks how many instances of that agent are currently running before spawning. If at max, the spawn is skipped (for crons) or queued (for pipeline steps/commands).

Example runbook:
```hcl
agent "doctor" {
  max_concurrency = 1
  run             = "claude --model sonnet"
  on_idle         = { action = "done" }
  prompt          = "Run diagnostics..."
}

cron "health_check" {
  interval = "30m"
  run      = { agent = "doctor" }
}
```

## Project Structure

Key files to modify:

```
crates/
├── runbook/src/
│   ├── agent.rs           # Add max_concurrency to AgentDef
│   ├── cron.rs            # Update CronDef doc comment (struct already uses RunDirective)
│   ├── parser.rs          # Relax cron validation to accept agent references
│   └── parser_tests/
│       └── cron.rs        # New test cases for agent crons
├── core/src/
│   ├── event.rs           # Generalize CronStarted/CronOnce/CronFired events
│   └── agent_run.rs       # (no changes — already has agent_name field)
├── storage/src/
│   └── state.rs           # Generalize CronRecord (pipeline_name → run target)
├── engine/src/
│   ├── runtime/
│   │   ├── mod.rs         # Add agent concurrency counting helper
│   │   ├── agent_run.rs   # Check max_concurrency before spawning standalone agents
│   │   ├── monitor.rs     # Check max_concurrency before spawning pipeline agents
│   │   └── handlers/
│   │       ├── cron.rs    # Handle both pipeline and agent targets on timer fire
│   │       ├── command.rs # Check max_concurrency before spawning
│   │       └── mod.rs     # Route new cron event variants
│   └── spawn.rs           # (no changes — already handles standalone agent spawning)
├── daemon/src/
│   └── listener/
│       ├── crons.rs       # Accept agent references in cron start/once validation
│       └── query_crons.rs # Display agent name instead of pipeline name when applicable
└── cli/src/
    └── commands/cron.rs   # (no changes expected — already generic)
```

## Dependencies

No new external dependencies. All required infrastructure already exists:
- `RunDirective::Agent { agent }` variant for parsing
- `spawn_standalone_agent()` for agent spawning
- `AgentRun` / `AgentRunStatus` for tracking
- `MaterializedState.agent_runs` for counting running instances

## Implementation Phases

### Phase 1: Generalize Cron Target Types

**Goal**: Allow cron definitions to reference agents, not just pipelines.

**1a. Relax parser validation** (`crates/runbook/src/parser.rs:340-368`)

The current validation block enforces pipeline-only:
```rust
// Current (parser.rs ~line 350):
let pipeline_name = match cron.run.pipeline_name() {
    Some(p) => p,
    None => {
        return Err(ParseError::InvalidFormat {
            location: format!("cron.{}.run", name),
            message: "cron run must reference a pipeline".to_string(),
        });
    }
};
```

Change to accept both pipeline and agent references:
```rust
match &cron.run {
    RunDirective::Pipeline { pipeline } => {
        if !runbook.pipelines.contains_key(pipeline.as_str()) {
            return Err(ParseError::InvalidFormat {
                location: format!("cron.{}.run", name),
                message: format!(
                    "references unknown pipeline '{}'; available pipelines: {}",
                    pipeline, sorted_keys(&runbook.pipelines),
                ),
            });
        }
    }
    RunDirective::Agent { agent } => {
        if !runbook.agents.contains_key(agent.as_str()) {
            return Err(ParseError::InvalidFormat {
                location: format!("cron.{}.run", name),
                message: format!(
                    "references unknown agent '{}'; available agents: {}",
                    agent, sorted_keys(&runbook.agents),
                ),
            });
        }
    }
    RunDirective::Shell(_) => {
        return Err(ParseError::InvalidFormat {
            location: format!("cron.{}.run", name),
            message: "cron run must reference a pipeline or agent".to_string(),
        });
    }
}
```

**1b. Update parser tests** (`crates/runbook/src/parser_tests/cron.rs`)

- Add `parse_hcl_cron_agent_valid` test: cron with `run = { agent = "..." }` parses successfully
- Update `error_cron_non_pipeline_run` test: error message changes to "must reference a pipeline or agent"
- Add `error_cron_unknown_agent` test: cron referencing nonexistent agent fails validation

**1c. Update CronDef doc comment** (`crates/runbook/src/cron.rs:9`)

Change "runs a pipeline on a timer interval" → "runs a pipeline or agent on a timer interval".

### Phase 2: Generalize Cron Events and State

**Goal**: Events and storage can represent agent-targeted crons, not just pipeline-targeted crons.

**2a. Generalize CronStarted event** (`crates/core/src/event.rs:264-273`)

The `pipeline_name` field is semantically wrong for agent crons. Add a `run_target` field alongside `pipeline_name` for backward compatibility during WAL replay:

```rust
#[serde(rename = "cron:started")]
CronStarted {
    cron_name: String,
    project_root: PathBuf,
    runbook_hash: String,
    interval: String,
    /// Deprecated: use run_target. Kept for WAL backward compat.
    #[serde(default)]
    pipeline_name: String,
    /// What this cron runs: "pipeline:name" or "agent:name"
    #[serde(default)]
    run_target: String,
    #[serde(default)]
    namespace: String,
},
```

Encode the target as `"pipeline:{name}"` or `"agent:{name}"`. Use a helper:
```rust
// In a utility module or on RunDirective:
fn run_target_string(directive: &RunDirective) -> String {
    match directive {
        RunDirective::Pipeline { pipeline } => format!("pipeline:{}", pipeline),
        RunDirective::Agent { agent } => format!("agent:{}", agent),
        RunDirective::Shell(cmd) => format!("shell:{}", cmd),
    }
}
```

For backward compat: if `run_target` is empty (old events), fall back to `pipeline_name`.

**2b. Generalize CronState** (`crates/engine/src/runtime/handlers/cron.rs:17-24`)

Replace `pipeline_name` with a `run_target` that captures the full `RunDirective`:

```rust
pub(crate) struct CronState {
    pub project_root: PathBuf,
    pub runbook_hash: String,
    pub interval: String,
    pub run_target: CronRunTarget,
    pub status: CronStatus,
    pub namespace: String,
}

pub(crate) enum CronRunTarget {
    Pipeline(String),
    Agent(String),
}
```

**2c. Generalize CronRecord** (`crates/storage/src/state.rs:111-127`)

Add `run_target` field alongside `pipeline_name`:

```rust
pub struct CronRecord {
    // ... existing fields ...
    pub pipeline_name: String,       // Keep for backward compat
    #[serde(default)]
    pub run_target: String,          // "pipeline:name" or "agent:name"
}
```

**2d. Update CronOnce event** — needs to handle agent targets too. Currently it has `pipeline_id`, `pipeline_name`, `pipeline_kind` fields. For agent targets, those aren't applicable. The simplest approach: add a parallel `agent_run_id` field and a `run_target` discriminator, keeping pipeline fields for backward compat. Or restructure:

```rust
#[serde(rename = "cron:once")]
CronOnce {
    cron_name: String,
    /// Set for pipeline targets
    #[serde(default)]
    pipeline_id: PipelineId,
    #[serde(default)]
    pipeline_name: String,
    #[serde(default)]
    pipeline_kind: String,
    /// Set for agent targets
    #[serde(default)]
    agent_run_id: Option<String>,
    #[serde(default)]
    agent_name: Option<String>,
    project_root: PathBuf,
    runbook_hash: String,
    #[serde(default)]
    run_target: String,
    #[serde(default)]
    namespace: String,
},
```

**2e. Update CronFired event** — add optional `agent_run_id`:
```rust
#[serde(rename = "cron:fired")]
CronFired {
    cron_name: String,
    #[serde(default)]
    pipeline_id: PipelineId,
    #[serde(default)]
    agent_run_id: Option<String>,
    #[serde(default)]
    namespace: String,
},
```

### Phase 3: Cron Timer Fires Agent

**Goal**: When a cron timer fires and the target is an agent, spawn a standalone agent instead of a pipeline.

**3a. Update `handle_cron_timer_fired()`** (`crates/engine/src/runtime/handlers/cron.rs:177-290`)

After determining the cron state, branch on `run_target`:

```rust
match &run_target {
    CronRunTarget::Pipeline(pipeline_name) => {
        // Existing pipeline creation logic (unchanged)
        let pipeline_id = PipelineId::new(UuidIdGen.next());
        // ... create_and_start_pipeline ...
    }
    CronRunTarget::Agent(agent_name) => {
        let runbook = self.cached_runbook(&runbook_hash)?;
        let agent_def = runbook.get_agent(agent_name)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_name.clone()))?
            .clone();

        let agent_run_id = AgentRunId::new(UuidIdGen.next());

        // Emit AgentRunCreated
        let creation_effects = vec![Effect::Emit {
            event: Event::AgentRunCreated {
                id: agent_run_id.clone(),
                agent_name: agent_name.clone(),
                command_name: format!("cron:{}", cron_name),
                namespace: namespace.clone(),
                cwd: project_root.clone(),
                runbook_hash: runbook_hash.clone(),
                vars: HashMap::new(),
                created_at_epoch_ms: self.clock().epoch_ms(),
            },
        }];
        result_events.extend(self.executor.execute_all(creation_effects).await?);

        // Spawn
        let spawn_events = self.spawn_standalone_agent(
            &agent_run_id,
            &agent_def,
            agent_name,
            &HashMap::new(),
            &project_root,
            &namespace,
        ).await?;
        result_events.extend(spawn_events);
    }
}
```

**3b. Update `handle_cron_once()`** (`crates/engine/src/runtime/handlers/cron.rs:132-174`)

Same branching logic: if agent target, spawn standalone agent instead of creating a pipeline.

**3c. Update `handle_cron_started()` log message** — show agent name if agent target.

### Phase 4: Daemon Validation for Agent Crons

**Goal**: The daemon listener accepts and validates agent-targeted cron requests.

**4a. Update `handle_cron_start()`** (`crates/daemon/src/listener/crons.rs:26-113`)

Replace the pipeline-only validation with a match on `RunDirective`:

```rust
match &cron_def.run {
    RunDirective::Pipeline { pipeline } => {
        if runbook.get_pipeline(pipeline).is_none() {
            return Ok(Response::Error { ... });
        }
    }
    RunDirective::Agent { agent } => {
        if runbook.get_agent(agent).is_none() {
            return Ok(Response::Error { ... });
        }
    }
    _ => return Ok(Response::Error { message: "cron run must reference a pipeline or agent" }),
}
```

Update the `CronStarted` event emission to include `run_target`.

**4b. Update `handle_cron_once()`** (`crates/daemon/src/listener/crons.rs:153-250`)

Same pattern. For agent targets, generate an `AgentRunId` instead of a `PipelineId` and emit the appropriate `CronOnce` event variant.

**4c. Update query display** (`crates/daemon/src/listener/query_crons.rs`)

Show the target type in cron list output (e.g., "agent:doctor" vs "pipeline:cleanup").

### Phase 5: Agent Max Concurrency

**Goal**: Add `max_concurrency` to agent definitions and enforce it before spawning.

**5a. Add field to AgentDef** (`crates/runbook/src/agent.rs:157-199`)

```rust
pub struct AgentDef {
    // ... existing fields ...
    /// Maximum concurrent instances of this agent. None = unlimited.
    #[serde(default)]
    pub max_concurrency: Option<u32>,
}
```

**5b. Add concurrency counting helper** (`crates/engine/src/runtime/mod.rs`)

```rust
impl Runtime {
    /// Count currently running (non-terminal) instances of an agent by name.
    pub(crate) fn count_running_agents(&self, agent_name: &str, namespace: &str) -> usize {
        self.lock_state(|state| {
            state.agent_runs.values()
                .filter(|ar| {
                    ar.agent_name == agent_name
                        && ar.namespace == namespace
                        && !ar.status.is_terminal()
                })
                .count()
        })
    }
}
```

This counts standalone agent runs. Pipeline-embedded agents also need counting — check `agent_pipelines` cross-referenced with pipeline state:

```rust
pub(crate) fn count_running_agents(&self, agent_name: &str, namespace: &str) -> usize {
    self.lock_state(|state| {
        // Count standalone agent runs
        let standalone = state.agent_runs.values()
            .filter(|ar| {
                ar.agent_name == agent_name
                    && ar.namespace == namespace
                    && !ar.status.is_terminal()
            })
            .count();

        // Count pipeline-embedded agents
        let in_pipeline = state.pipelines.values()
            .filter(|p| {
                p.namespace == namespace
                    && !p.status.is_terminal()
                    && p.current_agent_name().as_deref() == Some(agent_name)
            })
            .count();

        standalone + in_pipeline
    })
}
```

Note: `Pipeline` may need a helper `current_agent_name()` that looks at the current step's agent reference. If step history already tracks agent_name, use that. Otherwise, the runbook must be consulted. The simpler approach may be to only count `agent_runs` since standalone agent runs (from crons) are the primary use case.

**5c. Enforce in cron timer handler** (`crates/engine/src/runtime/handlers/cron.rs`)

Before spawning agent in `handle_cron_timer_fired()`:

```rust
CronRunTarget::Agent(agent_name) => {
    // Check max_concurrency
    if let Some(max) = agent_def.max_concurrency {
        let running = self.count_running_agents(agent_name, &namespace);
        if running >= max as usize {
            append_cron_log(
                self.logger.log_dir(),
                cron_name,
                &format!(
                    "skip: agent '{}' at max concurrency ({}/{})",
                    agent_name, running, max
                ),
            );
            // Reschedule timer but don't spawn
            // (timer rescheduling happens below regardless)
            return Ok(result_events);  // after rescheduling
        }
    }
    // ... spawn agent ...
}
```

**5d. Enforce in command handler** (`crates/engine/src/runtime/handlers/command.rs`)

For `RunDirective::Agent` in `handle_command()`, check before spawning:

```rust
if let Some(max) = agent_def.max_concurrency {
    let running = self.count_running_agents(&agent_name, namespace);
    if running >= max as usize {
        return Err(RuntimeError::InvalidRequest(format!(
            "agent '{}' at max concurrency ({}/{})",
            agent_name, running, max
        )));
    }
}
```

**5e. Enforce in pipeline step spawn** (`crates/engine/src/runtime/monitor.rs`)

For pipeline step agents, if at max concurrency, the step should wait rather than fail. This is more complex — the pipeline would need a "waiting for slot" state. For the initial implementation, treat the same as cron: skip/retry via the existing `on_dead` recovery mechanism. A full queuing system can be added later if needed.

**5f. Parser validation** (`crates/runbook/src/parser.rs`)

Validate `max_concurrency >= 1` if set:
```rust
if let Some(max) = agent.max_concurrency {
    if max == 0 {
        return Err(ParseError::InvalidFormat {
            location: format!("agent.{}.max_concurrency", agent_name),
            message: "max_concurrency must be >= 1".to_string(),
        });
    }
}
```

### Phase 6: Tests

**Goal**: Comprehensive test coverage for both features.

**6a. Parser tests** (`crates/runbook/src/parser_tests/cron.rs`)
- `parse_hcl_cron_agent_valid` — agent reference parses
- `parse_hcl_cron_agent_with_vars` — vars pass through (future)
- `error_cron_shell_run` — shell command rejected
- `error_cron_unknown_agent` — nonexistent agent rejected
- Update `error_cron_non_pipeline_run` — message now says "pipeline or agent"

**6b. Parser tests for max_concurrency** (`crates/runbook/src/parser_tests/agent.rs` or similar)
- `parse_agent_max_concurrency` — field parses correctly
- `parse_agent_max_concurrency_default` — defaults to None (unlimited)
- `error_agent_max_concurrency_zero` — zero rejected

**6c. Engine unit tests** (`crates/engine/`)
- `cron_timer_fires_agent` — cron with agent target spawns standalone agent
- `cron_once_agent` — one-shot cron with agent target works
- `cron_agent_concurrency_skip` — cron skips spawn when agent at max_concurrency
- `cron_agent_concurrency_respawns_after_complete` — spawn works again after previous instance completes
- `count_running_agents_standalone` — counting logic works for standalone runs
- `command_agent_max_concurrency_error` — command returns error at max

## Key Implementation Details

### Backward Compatibility

All WAL event changes use `#[serde(default)]` for new fields. Old events without `run_target` fall back to the `pipeline_name` field. The `CronRecord` similarly keeps `pipeline_name` and adds `run_target`. This ensures daemon restarts with existing WAL data work correctly.

### Agent Spawning in Cron Context

When a cron fires an agent, the `command_name` field on `AgentRun` is set to `"cron:{cron_name}"` to distinguish cron-triggered runs from command-triggered runs in logs and queries.

The `cwd` for cron-triggered agents is the `project_root` — same as where the runbook lives. This matches the behavior of `oj cron once` which runs in the project context.

### Concurrency Counting Strategy

Count only via `MaterializedState.agent_runs` (standalone runs). This is sufficient because:
- Cron-triggered agents are always standalone runs
- Pipeline-embedded agents are managed by pipeline concurrency, not agent concurrency
- The count is O(n) over agent_runs but this set is small in practice

If pipeline-embedded counting is needed later, add a `current_agent_name()` helper to `Pipeline` and include those in the count.

### Skip vs Queue Semantics

- **Crons**: Skip silently (log it). The next interval will try again. This prevents accumulation.
- **Commands**: Return an error. The user can retry manually.
- **Pipeline steps**: Out of scope for initial implementation. Pipeline step agents are typically unique per pipeline, making concurrency limits less relevant. Can be added later with a "waiting for slot" step state.

### Timer Rescheduling

When a cron fires and the agent spawn is skipped due to max_concurrency, the timer is still rescheduled for the next interval. The skip only affects the current tick.

## Verification Plan

1. **Parser**: `cargo test -p oj-runbook` — all cron and agent parser tests pass
2. **Engine**: `cargo test -p oj-engine` — cron-agent and concurrency tests pass
3. **Integration**: Manual test with a real runbook:
   ```hcl
   agent "doctor" {
     max_concurrency = 1
     run = "claude --model haiku"
     on_idle = { action = "done" }
     prompt = "Say hello and exit."
   }

   cron "health" {
     interval = "1m"
     run = { agent = "doctor" }
   }
   ```
   - `oj cron start health` — starts the cron
   - Verify agent spawns on first tick
   - Verify second tick is skipped while first agent is still running
   - Verify third tick spawns after first agent completes
4. **Full CI**: `make check` passes (fmt, clippy, build, test, deny)
