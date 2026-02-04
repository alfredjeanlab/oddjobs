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

/// Get the agent state directory in OJ state dir, creating it if needed.
fn agent_state_dir(agent_id: &str, state_dir: &Path) -> io::Result<PathBuf> {
    let agent_dir = state_dir.join("agents").join(agent_id);
    fs::create_dir_all(&agent_dir)?;

    Ok(agent_dir)
}

/// Get path to agent-specific settings file in OJ state directory
fn agent_settings_path(agent_id: &str, state_dir: &Path) -> io::Result<PathBuf> {
    Ok(agent_state_dir(agent_id, state_dir)?.join("claude-settings.json"))
}

/// Write agent prime script(s) to the state directory.
///
/// Returns a map of SessionStart matcher -> script path.
/// - For Script/Commands: single entry with empty matcher ("" = all sources)
/// - For PerSource: one entry per source (e.g., "startup" -> prime-startup.sh)
pub fn prepare_agent_prime(
    agent_id: &str,
    prime: &PrimeDef,
    vars: &HashMap<String, String>,
    state_dir: &Path,
) -> io::Result<HashMap<String, PathBuf>> {
    let agent_dir = agent_state_dir(agent_id, state_dir)?;
    let rendered = prime.render_per_source(vars);

    let mut paths = HashMap::new();
    for (source, content) in &rendered {
        let filename = if source.is_empty() {
            "prime.sh".to_string()
        } else {
            format!("prime-{}.sh", source)
        };
        let path = agent_dir.join(&filename);
        let script = format!("#!/usr/bin/env bash\nset -euo pipefail\n{}\n", content);
        fs::write(&path, &script)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }

        paths.insert(source.clone(), path);
    }

    Ok(paths)
}

/// Prepare settings file for an agent
///
/// Creates a settings file in the OJ state directory with the Stop hook configured.
/// Injects SessionStart hooks for prime scripts (one per source matcher).
/// Looks for project settings in the workspace directory (the runbook's init step
/// should have cloned or initialized the project there).
/// Returns the path to pass to claude via --settings.
pub fn prepare_agent_settings(
    agent_id: &str,
    workspace_path: &Path,
    prime_paths: &HashMap<String, PathBuf>,
    state_dir: &Path,
) -> io::Result<PathBuf> {
    let settings_path = agent_settings_path(agent_id, state_dir)?;

    // Load project settings from workspace (if they exist)
    let project_settings = workspace_path.join(".claude/settings.json");
    let mut settings: Value = if project_settings.exists() {
        let content = fs::read_to_string(&project_settings)?;
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    // Inject hooks (Stop + SessionStart per prime source)
    inject_hooks(&mut settings, agent_id, prime_paths);

    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings).unwrap_or_else(|_| "{}".to_string()),
    )?;

    Ok(settings_path)
}

/// Inject hooks into settings: Stop hook (always) and SessionStart hooks (one per prime source)
fn inject_hooks(settings: &mut Value, agent_id: &str, prime_paths: &HashMap<String, PathBuf>) {
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

    // Inject Notification hook for instant idle/permission detection
    let notification_hook_entry = json!({
        "matcher": "idle_prompt|permission_prompt",
        "hooks": [{
            "type": "command",
            "command": format!("oj agent hook notify --agent-id {}", agent_id)
        }]
    });

    hooks_obj.insert("Notification".to_string(), json!([notification_hook_entry]));

    // Inject PreToolUse hook for detecting plan/question tools
    let pretooluse_hook_entry = json!({
        "matcher": "ExitPlanMode|AskUserQuestion|EnterPlanMode",
        "hooks": [{
            "type": "command",
            "command": format!("oj agent hook pretooluse {}", agent_id)
        }]
    });
    hooks_obj.insert("PreToolUse".to_string(), json!([pretooluse_hook_entry]));

    // Inject SessionStart hooks — one entry per prime source
    if !prime_paths.is_empty() {
        let session_start_entries: Vec<Value> = prime_paths
            .iter()
            .map(|(matcher, path)| {
                json!({
                    "matcher": matcher,
                    "hooks": [{
                        "type": "command",
                        "command": format!("bash {}", path.display())
                    }]
                })
            })
            .collect();
        hooks_obj.insert("SessionStart".to_string(), json!(session_start_entries));
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
