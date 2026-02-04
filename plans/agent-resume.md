# Agent Resume

## Overview

Rename the `Recover` agent action to `Resume` and change its behavior from killing-and-fresh-spawning to using `claude --resume <session_id>`, preserving the agent's conversation history across restarts. Also add `on_error` as a valid trigger for resume (valuable for rate-limit recovery), enable resume for `on_error` triggers, and add an `oj agent resume` CLI command with `--kill` and `--all` flags.

## Project Structure

Modified and new files:

```
crates/
├── runbook/src/
│   ├── agent.rs                  # MODIFY: Recover → Resume enum, as_str, is_valid_for_trigger, invalid_reason
│   └── agent_tests.rs            # MODIFY: update test values and names
├── core/src/
│   ├── pipeline.rs               # MODIFY: add claude_session_id to StepRecord
│   ├── agent_run.rs              # MODIFY: add claude_session_id field
│   ├── event.rs                  # MODIFY: add claude_session_id to StepStarted, AgentRunStatusChanged
│   └── effect.rs                 # MODIFY: add resume_session_id to SpawnAgent
├── engine/src/
│   ├── monitor.rs                # MODIFY: Recover → Resume in ActionEffects, build_action_effects
│   ├── monitor_tests.rs          # MODIFY: update test names and assertions
│   ├── spawn.rs                  # MODIFY: thread resume_session_id into command construction
│   └── runtime/
│       ├── monitor.rs            # MODIFY: kill_and_respawn → kill_and_resume, use resume_session_id
│       ├── agent_run.rs          # MODIFY: standalone agent recover → resume handling
│       └── handlers/
│           └── agent.rs          # MODIFY: persist claude_session_id in step history
├── adapters/src/agent/
│   ├── claude.rs                 # MODIFY: pass --resume flag to claude spawn
│   └── watcher.rs                # MODIFY: extract session_id from JSONL on spawn
├── daemon/src/
│   ├── protocol.rs               # MODIFY: add AgentResume request variant
│   └── listener/
│       └── commands.rs           # MODIFY: handle AgentResume request
└── cli/src/commands/
    └── agent.rs                  # MODIFY: add Resume subcommand
```

## Dependencies

No new external dependencies. All functionality builds on existing infrastructure.

## Implementation Phases

### Phase 1: Rename Recover → Resume (pure rename, no behavior change)

Rename `AgentAction::Recover` to `AgentAction::Resume` and `ActionEffects::Recover` to `ActionEffects::Resume` across the codebase. Keep `"recover"` as a deprecated alias in parsing for backwards compatibility.

**Files:**

1. **`crates/runbook/src/agent.rs`** — Rename enum variant and update all references:
   - `AgentAction::Recover` → `AgentAction::Resume`
   - `as_str()`: return `"resume"` (primary name)
   - `is_valid_for_trigger()`: rename pattern match arms; also add `Resume` as valid for `OnError` (rate-limit recovery)
   - `invalid_reason()`: update messages from "recover" to "resume"
   - Serde: add `#[serde(alias = "recover")]` on the `Resume` variant for backwards compat

   ```rust
   #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
   #[serde(rename_all = "lowercase")]
   pub enum AgentAction {
       #[default]
       Nudge,
       Done,
       Fail,
       #[serde(alias = "recover")]
       Resume,    // Re-spawn with --resume, preserving conversation history
       Escalate,
       Gate,
   }
   ```

   For `is_valid_for_trigger`, enable `Resume` for `OnError`:
   ```rust
   ActionTrigger::OnError => matches!(
       self,
       AgentAction::Fail | AgentAction::Resume | AgentAction::Escalate | AgentAction::Gate
   ),
   ```

2. **`crates/runbook/src/agent_tests.rs`** — Update test assertions:
   - `AgentAction::Recover` → `AgentAction::Resume`
   - TOML test strings: keep `action = "recover"` tests (backwards compat) and add `action = "resume"` tests

3. **`crates/runbook/src/parser_tests/action_trigger.rs`** — Update `on_error_rejects_recover` → `on_error_accepts_resume` (since resume is now valid for on_error)

4. **`crates/engine/src/monitor.rs`** — Rename in `build_action_effects()`:
   - `AgentAction::Recover` → `AgentAction::Resume`
   - `ActionEffects::Recover` → `ActionEffects::Resume`

5. **`crates/engine/src/monitor_tests.rs`** — Rename tests:
   - `recover_returns_recover_effects` → `resume_returns_resume_effects`
   - `recover_with_message_replaces_prompt` → `resume_with_message_replaces_prompt`
   - `recover_with_append_appends_to_prompt` → `resume_with_append_appends_to_prompt`
   - Update assertions from `ActionEffects::Recover` → `ActionEffects::Resume`

6. **`crates/engine/src/runtime/monitor.rs`** — Rename:
   - `ActionEffects::Recover` match arm → `ActionEffects::Resume`
   - `kill_and_respawn` → `kill_and_resume` (method rename)

7. **`crates/engine/src/runtime/agent_run.rs`** — Rename:
   - `ActionEffects::Recover` match arm → `ActionEffects::Resume`

8. **`crates/engine/src/runtime/handlers/agent.rs`** — Update any "recovery" comments/docs to say "resume"

9. **Remaining grep hits** — Update comments, log messages, and doc strings in all 34 files that reference "recover" in the agent-action context. Non-agent uses of "recover" (e.g., WAL recovery, error recovery) should be left alone.

**Milestone:** `make check` passes. All existing tests pass with renamed variants. TOML `action = "recover"` still parses (via serde alias). `action = "resume"` also parses.

### Phase 2: Track Claude session ID

Store the Claude `--session-id` (the UUID passed to `claude --session-id <uuid>`) alongside agent state so it's available at resume time. Currently this value is generated in `spawn.rs` line 76 as `agent_id` and passed as `--session-id` to Claude, but it's only stored in the `StepStarted` event as `agent_id`. We need to ensure it's accessible when building resume effects.

**Files:**

1. **`crates/engine/src/monitor.rs`** — Add `resume_session_id: Option<String>` to `ActionEffects::Resume`:
   ```rust
   ActionEffects::Resume {
       kill_session: Option<String>,
       agent_name: String,
       input: HashMap<String, String>,
       resume_session_id: Option<String>,  // NEW: Claude --session-id from previous run
   },
   ```

2. **`crates/engine/src/monitor.rs`** `build_action_effects()` — Extract the previous agent's `agent_id` from `pipeline.step_history` (same pattern used in `handle_agent_resume` at `crates/engine/src/runtime/handlers/agent.rs:245`):
   ```rust
   AgentAction::Resume => {
       let mut new_inputs = input.clone();
       // ... existing message handling ...

       // Look up previous Claude session ID from step history
       let resume_session_id = pipeline
           .step_history
           .iter()
           .rfind(|r| r.name == pipeline.step)
           .and_then(|r| r.agent_id.clone());

       Ok(ActionEffects::Resume {
           kill_session: pipeline.session_id.clone(),
           agent_name: agent_def.name.clone(),
           input: new_inputs,
           resume_session_id,
       })
   }
   ```

3. **`crates/engine/src/monitor.rs`** — Extend `build_agent_run_action_effects()` similarly, pulling `agent_id` from `AgentRun.agent_id` for standalone agents.

4. **`crates/engine/src/spawn.rs`** — Add `resume_session_id: Option<String>` parameter to `build_spawn_effects`:
   - When `resume_session_id` is `Some(id)`, build the command with `--resume <id>` instead of the prompt argument
   - When `resume_session_id` is `None`, behave exactly as today (fresh spawn)

   Key logic change in command construction (around line 143):
   ```rust
   let command = if let Some(ref resume_id) = resume_session_id {
       // Resume mode: use --resume instead of passing prompt
       format!(
           "{} --resume {} --session-id {} --settings {}",
           base_command, resume_id, agent_id, settings_path.display()
       )
   } else if agent_def.run.contains("${prompt}") {
       // ... existing inline prompt logic ...
   } else {
       // ... existing prompt-as-argument logic ...
   };
   ```

   Note: `--session-id` is still passed alongside `--resume` so the new session gets a fresh UUID for log tracking, but `--resume` tells Claude to load conversation history from `resume_id`.

   **Important nuance from instructions:** When `ActionConfig::append()` is false and a message is set, the entire prompt is being replaced — this should NOT use `--resume`. Only use `--resume` when:
   - No message is set (bare resume), OR
   - `append` is true (message supplements existing context)

   Add a `use_resume: bool` field to `ActionEffects::Resume` to communicate this decision from `build_action_effects`:
   ```rust
   let use_resume = message.is_none() || action_config.append();
   ```

5. **`crates/engine/src/runtime/monitor.rs`** — Update `kill_and_resume` (renamed from `kill_and_respawn`) to pass `resume_session_id` through to `spawn_agent`:
   ```rust
   async fn kill_and_resume(
       &self,
       kill_session: Option<SessionId>,
       pipeline_id: &PipelineId,
       agent_name: &str,
       input: &HashMap<String, String>,
       resume_session_id: Option<String>,
   ) -> Result<Vec<Event>, RuntimeError> {
       if let Some(sid) = kill_session {
           self.executor
               .execute(Effect::KillSession { session_id: sid })
               .await?;
       }
       self.spawn_agent_with_resume(pipeline_id, agent_name, input, resume_session_id).await
   }
   ```

6. **`crates/engine/src/runtime/agent_run.rs`** — Same: pass `resume_session_id` through to `spawn_standalone_agent`.

**Milestone:** `make check` passes. Resume action now passes `--resume <session_id>` to Claude when a previous session ID is available and the prompt isn't being fully replaced. Falls back to fresh spawn when no session ID exists.

### Phase 3: Append message via `--resume` with `--message`

When resume is triggered with a message and `append = true`, the message should be passed via `claude --resume <id> "<message>"` rather than embedding it in the prompt template. This keeps the original prompt intact and adds the message as a new user turn in the conversation.

**Files:**

1. **`crates/engine/src/spawn.rs`** — When `resume_session_id` is set AND there's a resume message:
   ```rust
   let command = if let Some(ref resume_id) = resume_session_id {
       let resume_msg = input.get("resume_message").cloned().unwrap_or_default();
       if resume_msg.is_empty() {
           format!(
               "{} --resume {} --session-id {} --settings {}",
               base_command, resume_id, agent_id, settings_path.display()
           )
       } else {
           format!(
               "{} --resume {} --session-id {} --settings {} \"{}\"",
               base_command, resume_id, agent_id, settings_path.display(),
               escape_for_shell(&resume_msg)
           )
       }
   } else { /* ... existing fresh spawn ... */ };
   ```

2. **`crates/engine/src/monitor.rs`** — When building `ActionEffects::Resume` with `append = true` and a message, put the message in `input["resume_message"]` instead of modifying `input["prompt"]`:
   ```rust
   AgentAction::Resume => {
       let mut new_inputs = input.clone();
       let use_resume = message.is_none() || action_config.append();

       if let Some(msg) = message {
           if action_config.append() && use_resume {
               // Message will be passed as argument to --resume
               new_inputs.insert("resume_message".to_string(), msg.to_string());
           } else {
               // Replace mode: full prompt replacement, no --resume
               new_inputs.insert("prompt".to_string(), msg.to_string());
           }
       }
       // ...
   }
   ```

**Milestone:** `make check` passes. When `on_dead = { action = "resume", message = "Continue from where you left off", append = true }`, the agent is resumed with `claude --resume <prev_id> "Continue from where you left off"`.

### Phase 4: CLI command — `oj agent resume`

Add `oj agent resume <id>` with `--kill` and `--all` flags.

**Files:**

1. **`crates/cli/src/commands/agent.rs`** — Add `Resume` variant to `AgentCommand`:
   ```rust
   /// Resume a dead agent's session
   Resume {
       /// Agent ID (or prefix). Required unless --all is used.
       id: Option<String>,
       /// Force kill the current tmux session before resuming
       #[arg(long)]
       kill: bool,
       /// Resume all agents that have dead sessions
       #[arg(long)]
       all: bool,
   },
   ```

2. **`crates/daemon/src/protocol.rs`** — Add `AgentResume` request:
   ```rust
   /// Resume an agent (re-spawn with --resume to preserve conversation)
   AgentResume {
       /// Agent ID (full or prefix). Empty string for --all mode.
       agent_id: String,
       /// Force kill session before resuming
       kill: bool,
       /// Resume all dead agents
       all: bool,
   },
   ```

   Add corresponding response:
   ```rust
   /// Result of agent resume
   AgentResumed {
       /// Agents that were resumed (agent_id list)
       resumed: Vec<String>,
       /// Agents that were skipped with reasons
       skipped: Vec<(String, String)>,
   },
   ```

3. **`crates/daemon/src/listener/commands.rs`** — Handle `AgentResume`:
   - If `all`: iterate all agents with dead/exited sessions
   - If specific `id`: find the agent by ID/prefix
   - For each target agent:
     1. Look up the agent's last Claude `--session-id` (from `StepRecord.agent_id` or `AgentRun.agent_id`)
     2. If `kill` flag: emit `KillSession` effect for the agent's tmux session
     3. If not `kill`: check if tmux session is dead (skip if still alive)
     4. Emit a `PipelineResume` or `AgentRunResume` event to trigger the resume flow
   - The existing `handle_agent_resume` in `crates/engine/src/runtime/handlers/agent.rs` already handles the nudge-vs-recovery decision; we need to ensure the recovery path uses `--resume` (which Phase 2 achieves)

4. **`crates/cli/src/commands/agent.rs`** — Implement the CLI handler:
   ```rust
   AgentCommand::Resume { id, kill, all } => {
       if !all && id.is_none() {
           return Err(anyhow::anyhow!("Either provide an agent ID or use --all"));
       }
       let agent_id = id.unwrap_or_default();
       let result = client.agent_resume(&agent_id, kill, all).await?;
       // Display resumed/skipped agents
   }
   ```

5. **`crates/cli/src/client.rs`** — Add `agent_resume()` method that sends `Request::AgentResume` and parses the response.

**Milestone:** `oj agent resume <id>` kills a dead agent's tmux session and respawns with `claude --resume <prev_session_id>`. `oj agent resume --all` resumes all dead agents. `oj agent resume --all --kill` force-kills and resumes all agents.

### Phase 5: Tests

Add and update tests for the new behavior.

**Files:**

1. **`crates/engine/src/monitor_tests.rs`** — Update existing tests:
   - `resume_returns_resume_effects` — verify `resume_session_id` is populated from step history
   - `resume_with_message_replaces_prompt` — verify `use_resume` is false when append=false
   - `resume_with_append_appends_to_prompt` — verify `resume_message` key is set instead of modifying prompt
   - NEW: `resume_without_message_uses_resume_session` — verify bare resume includes session ID
   - NEW: `resume_with_no_prior_session_falls_back` — verify `resume_session_id` is None when no step history

2. **`crates/engine/src/runtime_tests/resume.rs`** — Update:
   - `resume_agent_dead_attempts_recovery` → verify spawn command includes `--resume`
   - NEW: `resume_agent_dead_no_session_history_fresh_spawn` — verify fresh spawn when no prior agent_id

3. **`crates/runbook/src/agent_tests.rs`** — Add:
   - `resume_action_parses` — `action = "resume"` parses to `AgentAction::Resume`
   - `recover_alias_parses_as_resume` — `action = "recover"` still parses to `AgentAction::Resume`
   - `resume_valid_for_on_error` — verify `AgentAction::Resume.is_valid_for_trigger(OnError)` is true

4. **`crates/runbook/src/parser_tests/action_trigger.rs`** — Update:
   - `on_error_rejects_recover` → `on_error_accepts_resume` (behavior change: resume is now valid for on_error)

**Milestone:** `make check` passes. Full test coverage for resume behavior including backwards compatibility.

## Key Implementation Details

### Session ID vs Agent ID nomenclature

The codebase uses `agent_id` (a UUID) as the `--session-id` argument to Claude. This is the identifier Claude uses to name its JSONL log file. When resuming, `--resume <session_id>` takes this same UUID. The field in `StepRecord` and `AgentRun` is already called `agent_id` and stores this UUID — no schema changes needed for storage.

### Resume vs fresh spawn decision matrix

| Condition | Spawn mode |
|-----------|------------|
| No message, previous session exists | `claude --resume <prev_id>` |
| Message with `append = true`, previous session exists | `claude --resume <prev_id> "<message>"` |
| Message with `append = false` (replace) | Fresh spawn with new prompt (no `--resume`) |
| No previous session (first run or missing log) | Fresh spawn (same as today) |

### on_error support for Resume

Currently `AgentAction::Recover` is rejected for `OnError` triggers. The `Resume` action should be valid for `OnError` because:
- Rate limits clear and the agent can pick up exactly where it left off
- Network errors are transient; resuming preserves context
- The agent process may still be alive (Claude exits on some errors, stays running for others)

When the agent is still alive on error, the existing nudge path in `handle_agent_resume` will be used. When it has exited, the resume path kicks in.

### Backwards compatibility

- `action = "recover"` in TOML/HCL continues to work via `#[serde(alias = "recover")]`
- WAL entries with `ActionEffects::Recover` need migration handling in deserialization — add `#[serde(alias = "Recover")]` on the `Resume` variant of `ActionEffects`
- The `as_str()` method returns `"resume"` for the primary name; any display/logging that previously showed "recover" will show "resume"

### Command construction for --resume

Claude Code's `--resume` flag takes a session ID (the UUID from a previous `--session-id`). The new command format:

```
claude --resume <prev_session_id> --session-id <new_session_id> --settings <path> ["<optional_message>"]
```

Both `--resume` and `--session-id` are passed: `--resume` tells Claude which conversation to load, `--session-id` tells it the ID for the new session's log file. The watcher monitors the new session's JSONL.

## Verification Plan

1. **Unit tests** — All renamed and new tests pass via `cargo test --all`
2. **Backwards compat** — Verify `action = "recover"` still parses correctly in TOML
3. **Integration** — Manual test with a runbook:
   ```toml
   [agent.worker]
   run = "claude"
   on_dead = { action = "resume", message = "Continue from where you left off.", append = true }
   on_error = [
     { match = "rate_limited", action = "resume", cooldown = "60s" },
   ]
   ```
   Kill the agent's tmux session, verify it respawns with `--resume` and the conversation history is preserved.
4. **CLI test** — `oj agent resume <id>` with a dead agent, verify it spawns with `--resume`
5. **CI** — `make check` passes (fmt, clippy, build, test, deny)
