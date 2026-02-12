// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent config file generation for coop

use oj_runbook::PrimeDef;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Write the coop agent-config file for an agent.
///
/// Creates `{state_dir}/agents/{agent_id}/agent-config.json` containing:
/// - `settings`: project settings (from workspace `.claude/settings.json`)
/// - `stop`: coop StopConfig (mode + prompt) derived from on_idle action
/// - `start`: coop StartConfig for priming (shell commands run on session start)
///
/// Returns the path to the agent-config file.
pub fn write_agent_config_file(
    agent_id: &str,
    workspace_path: &Path,
    prime: Option<&PrimeDef>,
    prime_vars: &HashMap<String, String>,
    on_idle_action: &str,
    on_idle_message: Option<&str>,
    state_dir: &Path,
) -> io::Result<PathBuf> {
    let agent_dir = oj_core::agent_dir(state_dir, agent_id);
    fs::create_dir_all(&agent_dir)?;
    let config_path = agent_dir.join("agent-config.json");

    // Build settings from project settings
    let project_settings = workspace_path.join(".claude/settings.json");
    let settings: Value = if project_settings.exists() {
        let content = fs::read_to_string(&project_settings)?;
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    // Build stop config from on_idle action
    let stop = build_stop_config(on_idle_action, on_idle_message);

    // Build config object
    let mut config = json!({
        "settings": settings,
        "stop": stop,
    });

    // Add start config if prime is defined
    if let Some(prime_def) = prime {
        config["start"] = build_start_config(prime_def, prime_vars);
    }

    fs::write(
        &config_path,
        serde_json::to_string_pretty(&config).unwrap_or_else(|_| "{}".to_string()),
    )?;

    Ok(config_path)
}

/// Build coop StartConfig JSON from a PrimeDef.
///
/// For single-source primes (Script/Commands), produces:
///   `{ "shell": ["set -euo pipefail\n<script>"] }`
///
/// For per-source primes (PerSource), produces:
///   `{ "event": { "<source>": { "shell": ["set -euo pipefail\n<script>"] }, ... } }`
fn build_start_config(prime: &PrimeDef, vars: &HashMap<String, String>) -> Value {
    let rendered = prime.render_per_source(vars);

    if rendered.len() == 1 && rendered.contains_key("") {
        // Single-source: top-level shell
        json!({ "shell": [format!("set -euo pipefail\n{}", rendered[""])] })
    } else {
        // Per-source: event map
        let event: Map<String, Value> = rendered
            .iter()
            .map(|(source, script)| {
                (source.clone(), json!({ "shell": [format!("set -euo pipefail\n{}", script)] }))
            })
            .collect();
        json!({ "event": event })
    }
}

/// Build coop StopConfig JSON from on_idle action.
///
/// Maps on_idle actions to coop stop modes:
/// - `done`/`fail` → `allow`: no interception; turn ends naturally
/// - `nudge`/`gate`/`resume` → `gate`: freeze agent, engine acts, then resolves
/// - `escalate` → `gate`: with prompt including AskUserQuestion instruction
/// - `auto` → `auto`: coop provides self-determination UI
fn build_stop_config(action: &str, message: Option<&str>) -> Value {
    let default_prompt = "Your turn was intercepted by the orchestrator. \
                          Wait for further instructions.";
    match action {
        "done" | "fail" => json!({ "mode": "allow" }),
        "nudge" | "gate" | "resume" => json!({
            "mode": "gate",
            "prompt": message.unwrap_or(default_prompt),
        }),
        "escalate" => {
            let base = message.unwrap_or(default_prompt);
            let prompt = format!("{}\n\nUse the AskUserQuestion tool before proceeding.", base);
            json!({
                "mode": "gate",
                "prompt": prompt,
            })
        }
        "auto" => json!({ "mode": "auto" }),
        _ => json!({ "mode": "allow" }),
    }
}

#[cfg(test)]
#[path = "agent_setup_tests.rs"]
mod tests;
