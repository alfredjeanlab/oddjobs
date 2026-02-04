# Env Command

## Overview

Add `oj env` — a CLI command for managing environment variables that are injected into all daemon-spawned processes (agents, shell steps). Variables are persisted to dotenv-style files in the state directory, scoped globally or per-project. The CLI reads/writes these files directly (no daemon IPC). The daemon reads them fresh on every process spawn, applying global vars first, then per-project overrides.

Primary use case: switching `CLAUDE_CODE_OAUTH_TOKEN` across all agents when credits run out.

## Project Structure

Files to create or modify:

```
crates/cli/src/commands/env.rs      # NEW: env command (set/list/unset)
crates/cli/src/commands/mod.rs      # Register env module
crates/cli/src/main.rs              # Add Commands::Env variant + dispatch
crates/engine/src/env.rs            # NEW: env file parsing + loading (shared by CLI and engine)
crates/engine/src/lib.rs            # Export env module
crates/engine/src/spawn.rs          # Inject user env vars into agent spawn
crates/engine/src/runtime/pipeline.rs # Inject user env vars into shell steps
crates/engine/src/executor.rs       # Inject user env vars into shell effects
```

## Dependencies

No new external dependencies. Uses `std::fs`, `std::io`, `std::path` for file I/O and `crates/cli/src/daemon_process.rs::daemon_dir()` for state directory resolution.

## Implementation Phases

### Phase 1: Env File Parser — `crates/engine/src/env.rs`

Create a module for reading/writing dotenv-style env files. This is shared code used by both the CLI (for `oj env` commands) and the engine (for injection at spawn time).

**File:** `crates/engine/src/env.rs`

**File format:** One `KEY=VALUE` per line. Empty lines and `#` comment lines are skipped. The value is everything after the first `=` (no quoting/unquoting — raw bytes).

```rust
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Resolve the path to the global env file.
pub fn global_env_path(state_dir: &Path) -> PathBuf {
    state_dir.join("env")
}

/// Resolve the path to a project-scoped env file.
pub fn project_env_path(state_dir: &Path, project: &str) -> PathBuf {
    state_dir.join(format!("env.{project}"))
}

/// Parse a dotenv-style file into ordered key-value pairs.
/// Returns an empty map if the file doesn't exist.
pub fn read_env_file(path: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e),
    };
    Ok(parse_env(&content))
}

/// Parse dotenv content string into key-value pairs.
fn parse_env(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].to_string();
            if !key.is_empty() {
                map.insert(key, value);
            }
        }
    }
    map
}

/// Write a BTreeMap back to a dotenv-style file.
/// Creates parent directories if needed. Removes the file if the map is empty.
pub fn write_env_file(path: &Path, vars: &BTreeMap<String, String>) -> std::io::Result<()> {
    if vars.is_empty() {
        // Remove file if empty (ignore NotFound)
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content: String = vars
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, content + "\n")
    }
}

/// Load merged environment: global vars first, then project overrides.
/// Returns a Vec of (key, value) pairs ready for env injection.
pub fn load_merged_env(state_dir: &Path, namespace: &str) -> Vec<(String, String)> {
    let mut merged = BTreeMap::new();

    // Global vars (base layer)
    if let Ok(global) = read_env_file(&global_env_path(state_dir)) {
        merged.extend(global);
    }

    // Project vars (override layer)
    if !namespace.is_empty() {
        if let Ok(project) = read_env_file(&project_env_path(state_dir, namespace)) {
            merged.extend(project);
        }
    }

    merged.into_iter().collect()
}
```

**Export:** Add `pub mod env;` to `crates/engine/src/lib.rs`.

**Tests:** Unit tests in `crates/engine/src/env_tests.rs`:
- `parse_env` with empty input, comments, valid pairs, value-with-equals
- `read_env_file` with missing file returns empty map
- `write_env_file` round-trip
- `write_env_file` with empty map removes file
- `load_merged_env` with global-only, project-only, and override behavior

### Phase 2: CLI Command — `crates/cli/src/commands/env.rs`

Add the `oj env` subcommand with `set`, `list`, and `unset` subcommands. This command is purely file-based — it does not connect to the daemon.

**File:** `crates/cli/src/commands/env.rs`

```rust
use anyhow::{bail, Result};
use clap::{Args, Subcommand};

use crate::daemon_process;
use crate::output::OutputFormat;

#[derive(Args)]
pub struct EnvArgs {
    #[command(subcommand)]
    pub command: EnvCommand,
}

#[derive(Subcommand)]
pub enum EnvCommand {
    /// Set an environment variable
    Set {
        /// Variable name
        key: String,
        /// Variable value
        value: String,
        /// Set globally (all projects)
        #[arg(long, conflicts_with = "project")]
        global: bool,
        /// Set for a specific project
        #[arg(long)]
        project: Option<String>,
    },
    /// List environment variables
    List {
        /// Show only global variables
        #[arg(long, conflicts_with = "project")]
        global: bool,
        /// Show only variables for a specific project
        #[arg(long)]
        project: Option<String>,
    },
    /// Remove an environment variable
    Unset {
        /// Variable name
        key: String,
        /// Remove from global scope
        #[arg(long, conflicts_with = "project")]
        global: bool,
        /// Remove from a specific project
        #[arg(long)]
        project: Option<String>,
    },
}
```

**Handler logic:**

- `set`: Require exactly one of `--global` or `--project`. Load the appropriate env file via `oj_engine::env::read_env_file`, insert the key-value pair, write back with `write_env_file`.
- `unset`: Same scope requirement. Load, remove key, write back. Print warning if key wasn't present.
- `list`:
  - `--global`: Show only global vars.
  - `--project <name>`: Show only that project's vars.
  - Neither flag: Show all — global vars and all `env.*` files in state_dir. Use `std::fs::read_dir` to discover `env.*` files.

**Text output format** (for `list`):

```
# global
CLAUDE_CODE_OAUTH_TOKEN=sk-ant-...
OJ_SOME_VAR=value

# project: oddjobs
PROJECT_SPECIFIC=abc
```

**JSON output format:**

```json
{
  "global": { "CLAUDE_CODE_OAUTH_TOKEN": "sk-ant-..." },
  "projects": {
    "oddjobs": { "PROJECT_SPECIFIC": "abc" }
  }
}
```

For scoped list (`--global` or `--project`), output just the flat key-value pairs.

**Registration:**

1. `crates/cli/src/commands/mod.rs` — add `pub mod env;`
2. `crates/cli/src/main.rs` — add to imports and `Commands` enum:

```rust
// In the use block:
use commands::{
    agent, cron, daemon, decision, emit, env, pipeline, project, /* ... */
};

// In the Commands enum:
/// Environment variable management
Env(env::EnvArgs),
```

3. `crates/cli/src/main.rs` — add dispatch (before the daemon-early-return, since env doesn't need a daemon either):

```rust
// Handle env command separately (doesn't need client connection)
if let Commands::Env(args) = command {
    return env::handle(args.command, format);
}
```

This dispatch should go right after the existing `Commands::Daemon` early return at line 185-187, since `oj env` is also a daemon-independent command.

### Phase 3: Inject Env Vars into Agent Spawns

Modify `build_spawn_effects` in `crates/engine/src/spawn.rs` to load user env vars and prepend them to the agent's env vec.

**File:** `crates/engine/src/spawn.rs`

Insert after the existing env setup (after the `CLAUDE_CODE_OAUTH_TOKEN` forwarding block, around line 225) and before the cwd computation:

```rust
// Inject user-managed env vars (global + per-project).
// Read fresh on every spawn so changes take effect immediately.
let user_env = crate::env::load_merged_env(state_dir, ctx.namespace);
for (key, value) in user_env {
    // Don't override env vars already set by the agent definition or system.
    // Agent-defined and system vars (OJ_NAMESPACE, OJ_STATE_DIR, etc.)
    // take precedence over user env files.
    if !env.iter().any(|(k, _)| k == &key) {
        env.push((key, value));
    }
}
```

**Precedence order (highest to lowest):**
1. Agent definition env vars (`agent_def.build_env`)
2. System vars (`OJ_NAMESPACE`, `OJ_STATE_DIR`, `OJ_DAEMON_BINARY`)
3. Forwarded vars (`CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_OAUTH_TOKEN`)
4. User env files (project overrides global)

This means `oj env set CLAUDE_CODE_OAUTH_TOKEN ...` will be overridden if the agent definition or the daemon's own environment already sets it. That's the correct behavior — user env files fill in defaults, not force overrides. However, since the main use case is `CLAUDE_CODE_OAUTH_TOKEN`, and the forwarding code at lines 216-224 only runs when the daemon's own environment has the var set, user env files will take effect when the daemon env doesn't have it.

**Important nuance:** The existing `CLAUDE_CODE_OAUTH_TOKEN` forwarding (lines 216-224) checks `!env.iter().any(|(k, _)| k == "CLAUDE_CODE_OAUTH_TOKEN")` and then reads from `std::env::var`. If the daemon process itself doesn't have the token set, that block is a no-op, and the user env file value will be injected by the new code. This is the desired behavior for the primary use case.

### Phase 4: Inject Env Vars into Shell Steps

Shell steps also need user env vars. There are two injection points:

**1. Pipeline shell steps** — `crates/engine/src/runtime/pipeline.rs`

Around line 117, where `shell_env` is constructed:

```rust
let mut shell_env = HashMap::new();
if !pipeline.namespace.is_empty() {
    shell_env.insert("OJ_NAMESPACE".to_string(), pipeline.namespace.clone());
}

// Inject user-managed env vars (global + per-project)
let user_env = crate::env::load_merged_env(&self.state_dir, &pipeline.namespace);
for (key, value) in user_env {
    if !shell_env.contains_key(&key) {
        shell_env.insert(key, value);
    }
}
```

**2. Shell effect execution** — `crates/engine/src/executor.rs`

The `Effect::Shell` handler at line 300-373 spawns `tokio::process::Command` with `.envs(&env)`. The env HashMap is passed through from the effect. Since we inject vars at the point where the effect is _created_ (in `pipeline.rs`), the executor just passes them through — no changes needed in executor.rs itself.

Similarly, `Effect::PollQueue` and `Effect::TakeQueueItem` don't use the env injection path. These are worker-internal commands. If worker env injection is desired later, it can be added.

### Phase 5: Tests

**Unit tests** (in `crates/engine/src/env_tests.rs`):
- Round-trip: write then read, verify equality
- Comment and blank line handling
- Values containing `=` signs (e.g., `API_KEY=abc=def123`)
- Missing file returns empty BTreeMap
- Empty map removes file
- Merge precedence: project overrides global, both present, only one present

**Spawn tests** (in `crates/engine/src/spawn_tests.rs`):
- Add a test that creates temp env files, calls `build_spawn_effects`, and verifies user env vars appear in the `Effect::SpawnAgent.env` vec
- Verify precedence: agent-defined vars win over user env vars

**CLI integration test** (optional, in `tests/`):
- `oj env set FOO bar --global` then `oj env list --global` shows `FOO=bar`
- `oj env unset FOO --global` then `oj env list --global` doesn't show `FOO`
- `oj env set FOO bar --project test` creates `env.test` file

## Key Implementation Details

### File Format

The env files use a minimal dotenv format:
```
# Comment lines start with #
KEY=value
ANOTHER_KEY=value with spaces
TOKEN=abc=def=123
```

No quoting. The value is everything after the first `=`. This keeps parsing trivial and avoids shell-quoting ambiguities. Users who need complex values can use them directly — the value is passed as-is to the process environment, not through shell interpretation.

### Precedence

```
Agent definition env  >  System vars  >  Forwarded vars  >  User project env  >  User global env
```

User env files are the lowest priority. They fill in defaults that aren't already provided by the agent definition or system. Within user env, project vars override global vars (applied by `load_merged_env` which inserts global first, then project on top).

### State Directory Resolution

The CLI resolves `state_dir` via `daemon_process::daemon_dir()` (respects `OJ_STATE_DIR`, `XDG_STATE_HOME`, falls back to `~/.local/state/oj`). The engine has `state_dir` available as `self.state_dir` in the runtime. Both paths use the same directory, so env files are shared.

### No Daemon IPC

The `oj env` commands are purely file-based. They don't need the daemon running. This is similar to how `oj daemon` is dispatched before daemon client creation in `main.rs`. The env command should be dispatched similarly.

### File Discovery for `oj env list`

When listing all vars (no `--global` or `--project` flag), the CLI scans the state directory for files matching the pattern `env` (global) and `env.*` (per-project). The project name is extracted from the filename suffix after `env.`.

## Verification Plan

1. **Phase 1:** Run `cargo test -p oj-engine` — env parser unit tests pass
2. **Phase 2:** Run `cargo build -p oj-cli` — env command compiles. Manual test: `oj env set FOO bar --global && oj env list --global` shows the var. `oj env unset FOO --global && oj env list --global` removes it.
3. **Phase 3:** Run `cargo test -p oj-engine` — spawn tests verify env injection. Manual test: set `CLAUDE_CODE_OAUTH_TOKEN` via `oj env set`, run a pipeline, verify the agent sees the token.
4. **Phase 4:** Verify shell steps also receive injected vars — add a pipeline with a shell step that echoes an env var set via `oj env`.
5. **Full:** Run `make check` — fmt, clippy, build, all tests pass.
