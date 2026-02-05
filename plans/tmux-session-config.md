# Tmux Session Config

Add an optional `session "tmux" { ... }` block to agent definitions for adapter-specific session configuration (status bar colors, titles, status text). Apply sensible defaults for all agents.

## Project Structure

```
crates/
├── runbook/src/
│   ├── agent.rs          # Add SessionConfig, TmuxSessionConfig structs to AgentDef
│   └── agent_tests.rs    # Parser round-trip tests for session block
├── core/src/
│   └── effect.rs         # Add session_config to Effect::SpawnAgent
├── adapters/src/
│   ├── agent/
│   │   ├── mod.rs        # Add session_config to AgentSpawnConfig
│   │   ├── claude.rs     # Thread session config through spawn, apply after session creation
│   │   └── fake.rs       # Accept and record session config in FakeAgentAdapter
│   └── session/
│       ├── mod.rs         # Add configure_session method to SessionAdapter trait
│       ├── tmux.rs        # Implement tmux set-option calls
│       ├── fake.rs        # Record configure calls in FakeSessionAdapter
│       └── noop.rs        # No-op implementation
├── engine/src/
│   ├── spawn.rs          # Thread session config from AgentDef into Effect::SpawnAgent
│   └── executor.rs       # Thread session config from effect into AgentSpawnConfig
```

## Dependencies

No new external dependencies. Uses existing `tokio::process::Command` for tmux calls.

## Implementation Phases

### Phase 1: Runbook Data Model

Add session config types to the runbook crate and parse them from HCL/TOML.

**Files:** `crates/runbook/src/agent.rs`, `crates/runbook/src/agent_tests.rs`

1. Define `TmuxSessionConfig` struct with optional fields for tmux-specific options:

```rust
/// Status bar text configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionStatusConfig {
    pub left: Option<String>,
    pub right: Option<String>,
}

/// Tmux-specific session configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TmuxSessionConfig {
    /// Status bar background color (red, green, blue, cyan, magenta, yellow, white)
    #[serde(default)]
    pub color: Option<String>,
    /// Window title string
    #[serde(default)]
    pub title: Option<String>,
    /// Status bar left/right text
    #[serde(default)]
    pub status: Option<SessionStatusConfig>,
}
```

2. Add a `session` field to `AgentDef` as a generic map from provider label to config:

```rust
/// Adapter-specific session configuration (e.g., session "tmux" { ... })
/// Keyed by provider name. Unknown providers are ignored.
#[serde(default)]
pub session: HashMap<String, TmuxSessionConfig>,
```

Note: Since the only provider is `tmux` today, we can type-erase later if needed. For now, parsing directly into `TmuxSessionConfig` works because the `hcl` crate deserializes labeled blocks as `HashMap<label, T>`. The `#[serde(default)]` ensures the field is optional.

3. Update `Default for AgentDef` to include `session: HashMap::new()`.

4. Add parser tests:
   - Round-trip HCL with `session "tmux" { color = "cyan" }` parses correctly
   - HCL with `session "tmux" { status { left = "foo" right = "bar" } }` parses correctly
   - Agent with no session block parses with empty map (backward compatible)
   - Unknown provider label (e.g., `session "zellij" { ... }`) parses without error (ignored at adapter level)

5. Add validation in `parser.rs` for tmux session config:
   - Validate `color` is one of: red, green, blue, cyan, magenta, yellow, white (if present)
   - Location: `agent.<name>.session.tmux.color`

### Phase 2: Thread Session Config Through the Effect System

Pass session config from the runbook definition through the effect and spawn config types to the adapter layer.

**Files:** `crates/core/src/effect.rs`, `crates/engine/src/spawn.rs`, `crates/engine/src/executor.rs`, `crates/adapters/src/agent/mod.rs`

1. Add a `session_config` field to `Effect::SpawnAgent`:

```rust
SpawnAgent {
    // ... existing fields ...
    /// Adapter-specific session configuration (provider -> config)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    session_config: HashMap<String, serde_json::Value>,
}
```

Use `serde_json::Value` at the effect level so the core crate stays generic. The runbook crate serializes `TmuxSessionConfig` to JSON, and the adapter deserializes it. This keeps `oj_core` free of adapter-specific types.

2. Update `build_spawn_effects()` in `crates/engine/src/spawn.rs` to serialize the session config from `AgentDef` and include it in the `SpawnAgent` effect. Also inject defaults for `status.left` and `status.right` here, since this is where all the context variables (namespace, job name, agent name, agent_id) are available:

```rust
// Build session config with defaults
let mut session_config: HashMap<String, serde_json::Value> = HashMap::new();
if let Some(tmux_config) = agent_def.session.get("tmux") {
    session_config.insert("tmux".to_string(), serde_json::to_value(tmux_config).unwrap());
}
// Always ensure tmux has default status bars (even without explicit session block)
let tmux_value = session_config
    .entry("tmux".to_string())
    .or_insert_with(|| serde_json::json!({}));
if let serde_json::Value::Object(ref mut map) = tmux_value {
    // Inject default status if not explicitly set
    let status = map.entry("status").or_insert_with(|| serde_json::json!({}));
    if let serde_json::Value::Object(ref mut status_map) = status {
        let short_id = &agent_id[..8]; // first 8 chars of UUID
        status_map
            .entry("left")
            .or_insert_with(|| serde_json::json!(format!("{} {}/{}", ctx.namespace, ctx.name, agent_name)));
        status_map
            .entry("right")
            .or_insert_with(|| serde_json::json!(short_id));
    }
}
```

3. Add `session_config` to `AgentSpawnConfig`:

```rust
pub struct AgentSpawnConfig {
    // ... existing fields ...
    /// Adapter-specific session configuration (provider -> config as JSON)
    pub session_config: HashMap<String, serde_json::Value>,
}
```

4. Update `Executor::execute_inner()` for the `SpawnAgent` match arm to pass `session_config` from the effect into `AgentSpawnConfig`.

5. Update `Effect::fields()` in the `TracedEffect` impl: no need to add session_config to tracing fields (it's diagnostic noise).

6. Update `FakeAgentAdapter` to accept the new field (it already ignores most config fields).

### Phase 3: Session Adapter — Configure Method

Add a `configure` method to `SessionAdapter` for post-creation session styling.

**Files:** `crates/adapters/src/session/mod.rs`, `crates/adapters/src/session/tmux.rs`, `crates/adapters/src/session/fake.rs`, `crates/adapters/src/session/noop.rs`

1. Add `configure` to the `SessionAdapter` trait with a default no-op:

```rust
/// Apply configuration to an existing session (styling, status bar, etc.)
/// Default implementation is a no-op.
async fn configure(
    &self,
    _id: &str,
    _config: &serde_json::Value,
) -> Result<(), SessionError> {
    Ok(())
}
```

2. Implement `TmuxAdapter::configure()`:

```rust
async fn configure(
    &self,
    id: &str,
    config: &serde_json::Value,
) -> Result<(), SessionError> {
    let tmux_config: TmuxSessionConfig = serde_json::from_value(config.clone())
        .map_err(|e| SessionError::CommandFailed(format!("invalid tmux config: {}", e)))?;

    // Apply status bar background color
    if let Some(ref color) = tmux_config.color {
        run_tmux_set_option(id, "status-style", &format!("bg={},fg=black", color)).await?;
    }

    // Apply window title
    if let Some(ref title) = tmux_config.title {
        run_tmux_set_option(id, "set-titles", "on").await?;
        run_tmux_set_option(id, "set-titles-string", title).await?;
    }

    // Apply status bar text
    if let Some(ref status) = tmux_config.status {
        if let Some(ref left) = status.left {
            run_tmux_set_option(id, "status-left", &format!(" {} ", left)).await?;
        }
        if let Some(ref right) = status.right {
            run_tmux_set_option(id, "status-right", &format!(" {} ", right)).await?;
        }
    }

    Ok(())
}
```

Helper function:

```rust
async fn run_tmux_set_option(session_id: &str, option: &str, value: &str) -> Result<(), SessionError> {
    let output = Command::new("tmux")
        .args(["set-option", "-t", session_id, option, value])
        .output()
        .await
        .map_err(|e| SessionError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(session_id, option, value, stderr = %stderr, "tmux set-option failed");
        // Non-fatal: session works even if styling fails
    }

    Ok(())
}
```

3. Update `FakeSessionAdapter`: Add a `Configure` variant to `SessionCall` and record calls. No actual tmux operations.

4. Update `NoOpSessionAdapter`: Default no-op is sufficient (trait default).

### Phase 4: Wire It Together in ClaudeAgentAdapter

Call `configure` after session spawn in the Claude agent adapter.

**Files:** `crates/adapters/src/agent/claude.rs`

1. After the `self.sessions.spawn()` call succeeds (line 376-380 of `claude.rs`), call configure with the tmux-specific session config:

```rust
// 5a. Apply session configuration (status bar, colors, title)
if let Some(tmux_config) = config.session_config.get("tmux") {
    if let Err(e) = self.sessions.configure(&spawned_id, tmux_config).await {
        tracing::warn!(
            agent_id = %config.agent_id,
            error = %e,
            "failed to configure session (non-fatal)"
        );
    }
}
```

This goes right after step 5 (session spawn) and before step 5b (bypass permissions prompt). Session styling is purely cosmetic, so errors are logged as warnings and don't fail the spawn.

### Phase 5: Tests

Add unit tests across all layers.

**Files:** Various `*_tests.rs` files

1. **Runbook parser tests** (`crates/runbook/src/agent_tests.rs`):
   - Parse HCL with session block, verify fields
   - Parse HCL without session block, verify empty map
   - Parse with unknown provider, verify no error
   - Validate color rejects invalid values
   - Parse with nested status block

2. **Spawn effects tests** (`crates/engine/src/spawn_tests.rs`):
   - Verify `build_spawn_effects()` includes default status left/right when no session block
   - Verify explicit session config overrides defaults
   - Verify namespace/job/step appear in default status-left

3. **Tmux adapter tests** (`crates/adapters/src/session/tmux_tests.rs`):
   - Test `configure` with full config (color + title + status) — unit test with mock (or integration test with real tmux)
   - Test `configure` with partial config (only color)
   - Test `configure` with empty config (no-op)

4. **Executor tests** (`crates/engine/src/executor_tests.rs`):
   - Verify session_config passes through from effect to AgentSpawnConfig

5. **Effect serialization tests** (`crates/core/src/effect_tests.rs`):
   - Verify SpawnAgent round-trips with session_config
   - Verify empty session_config is skipped in serialization

## Key Implementation Details

### Design Decisions

1. **Generic at core, typed at edges**: `Effect::SpawnAgent` carries `serde_json::Value` so `oj_core` doesn't depend on adapter-specific types. The runbook crate defines the typed `TmuxSessionConfig`; the adapter deserializes from `Value`.

2. **Defaults injected in spawn.rs**: The engine's `build_spawn_effects()` is the natural place to assemble defaults because it has access to all context variables (namespace, job name, step name, agent ID). The adapter just applies whatever config it receives.

3. **Non-fatal styling**: Session configuration errors are logged as warnings, never fail the spawn. An agent with broken styling is better than no agent.

4. **Provider-keyed map**: `session` is `HashMap<String, TmuxSessionConfig>` at the runbook level. This matches HCL labeled block syntax (`session "tmux" { ... }`) and allows future providers without schema changes. Unknown providers are silently ignored.

5. **Status bar defaults**: Every agent gets default status bars showing `<namespace> <job>/<step>` on the left and the short agent ID (first 8 hex chars) on the right. Explicit values in the session block override these defaults.

### Color Validation

Valid colors: `red`, `green`, `blue`, `cyan`, `magenta`, `yellow`, `white`. Validated at parse time in `parser.rs`. These are standard tmux named colors that work across terminals.

### HCL Syntax

```hcl
agent "mayor" {
  run = "claude --dangerously-skip-permissions"

  session "tmux" {
    color = "cyan"
    title = "mayor"
    status {
      left  = "myproject merge/check"
      right = "custom-id"
    }
  }

  prompt = "..."
}
```

## Verification Plan

1. **Unit tests pass**: `cargo test --all` — all new and existing tests pass
2. **Clippy clean**: `cargo clippy --all -- -D warnings`
3. **Format clean**: `cargo fmt --all`
4. **Build succeeds**: `cargo build --all`
5. **Manual smoke test**: Create a runbook with `session "tmux" { color = "green" }`, run it, and verify the tmux status bar turns green
6. **Backward compatibility**: Existing runbooks with no session block continue to work unchanged, and now get default status bars
7. **Full check**: `make check`
