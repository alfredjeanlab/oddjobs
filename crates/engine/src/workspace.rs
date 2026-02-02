// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Workspace preparation for agent execution

use oj_runbook::PrimeDef;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Prepare workspace directory for agent execution
///
/// Creates the workspace directory if needed. Does NOT write settings -
/// settings are now written to the OJ state directory via `prepare_agent_settings()`.
pub fn prepare_for_agent(workspace_path: &Path) -> io::Result<()> {
    // Just ensure workspace exists - no longer write settings here
    fs::create_dir_all(workspace_path)?;
    fs::create_dir_all(workspace_path.join(".claude"))?;
    Ok(())
}

/// Get path to agent-specific settings file in OJ state directory
fn agent_settings_path(agent_id: &str) -> io::Result<PathBuf> {
    let state_dir = std::env::var("OJ_STATE_DIR").unwrap_or_else(|_| {
        format!(
            "{}/.local/state/oj",
            std::env::var("HOME").unwrap_or_default()
        )
    });

    let agent_dir = PathBuf::from(&state_dir).join("agents").join(agent_id);
    fs::create_dir_all(&agent_dir)?;

    Ok(agent_dir.join("claude-settings.json"))
}

/// Write the agent's prime script to the state directory.
///
/// Returns the path to prime.sh if the agent has a prime field.
pub fn prepare_agent_prime(
    agent_id: &str,
    prime: &PrimeDef,
    vars: &HashMap<String, String>,
) -> io::Result<PathBuf> {
    let state_dir = std::env::var("OJ_STATE_DIR").unwrap_or_else(|_| {
        format!(
            "{}/.local/state/oj",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    let agent_dir = PathBuf::from(&state_dir).join("agents").join(agent_id);
    fs::create_dir_all(&agent_dir)?;

    let prime_path = agent_dir.join("prime.sh");
    let content = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\n{}\n",
        prime.render(vars)
    );
    fs::write(&prime_path, &content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&prime_path, fs::Permissions::from_mode(0o755))?;
    }

    Ok(prime_path)
}

/// Prepare settings file for an agent
///
/// Creates a settings file in the OJ state directory with the Stop hook configured.
/// Optionally injects a SessionStart hook for prime scripts.
/// Looks for project settings in the workspace directory (the runbook's init step
/// should have cloned or initialized the project there).
/// Returns the path to pass to claude via --settings.
pub fn prepare_agent_settings(
    agent_id: &str,
    workspace_path: &Path,
    prime_path: Option<&Path>,
) -> io::Result<PathBuf> {
    let settings_path = agent_settings_path(agent_id)?;

    // Load project settings from workspace (if they exist)
    let project_settings = workspace_path.join(".claude/settings.json");
    let mut settings: Value = if project_settings.exists() {
        let content = fs::read_to_string(&project_settings)?;
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    // Inject hooks (Stop + optional SessionStart)
    inject_hooks(&mut settings, agent_id, prime_path);

    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings).unwrap_or_else(|_| "{}".to_string()),
    )?;

    Ok(settings_path)
}

/// Inject hooks into settings: Stop hook (always) and SessionStart hook (if prime_path provided)
fn inject_hooks(settings: &mut Value, agent_id: &str, prime_path: Option<&Path>) {
    // Claude Code hooks require nested structure with matcher and hooks fields
    let stop_hook_entry = json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": format!("oj agent hook stop {}", agent_id)
        }]
    });

    // Ensure settings is an object
    if !settings.is_object() {
        *settings = json!({});
    }

    let Some(settings_obj) = settings.as_object_mut() else {
        return;
    };

    // Get or create hooks object
    let hooks = settings_obj.entry("hooks").or_insert_with(|| json!({}));

    // Ensure hooks is an object
    if !hooks.is_object() {
        *hooks = json!({});
    }

    let Some(hooks_obj) = hooks.as_object_mut() else {
        return;
    };

    // Always set the Stop hook (we control this settings file)
    hooks_obj.insert("Stop".to_string(), json!([stop_hook_entry]));

    // Inject Notification hooks for instant state detection
    let idle_hook_entry = json!({
        "matcher": "idle_prompt",
        "hooks": [{
            "type": "command",
            "command": format!("oj emit agent:idle --agent {}", agent_id)
        }]
    });

    let permission_hook_entry = json!({
        "matcher": "permission_prompt",
        "hooks": [{
            "type": "command",
            "command": format!("oj emit agent:prompt --agent {} --type permission", agent_id)
        }]
    });

    hooks_obj.insert(
        "Notification".to_string(),
        json!([idle_hook_entry, permission_hook_entry]),
    );

    // Inject SessionStart hook if prime path is provided
    if let Some(path) = prime_path {
        let session_start_entry = json!({
            "matcher": "",
            "hooks": [{
                "type": "command",
                "command": format!("bash {}", path.display())
            }]
        });
        hooks_obj.insert("SessionStart".to_string(), json!([session_start_entry]));
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
