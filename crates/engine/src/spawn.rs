// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent spawning functionality

use crate::error::RuntimeError;
use crate::ExecuteError;
use oj_core::{AgentId, Effect, Pipeline, PipelineId, TimerId};
use oj_runbook::AgentDef;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

/// Liveness check interval (30 seconds)
pub const LIVENESS_INTERVAL: Duration = Duration::from_secs(30);

/// Escape characters that have special meaning in shell double-quoted strings.
///
/// When a prompt is embedded in a command like `claude "${prompt}"`, characters
/// like backticks and dollar signs would be interpreted by the shell. This
/// function escapes them so they're treated literally.
///
/// Characters escaped:
/// - Backslash `\` → `\\`
/// - Backtick `` ` `` → `` \` ``
/// - Dollar sign `$` → `\$`
/// - Double quote `"` → `\"`
fn escape_for_shell_double_quotes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '`' => result.push_str("\\`"),
            '$' => result.push_str("\\$"),
            '"' => result.push_str("\\\""),
            _ => result.push(c),
        }
    }
    result
}

/// Spawn an agent for a pipeline
///
/// Returns the effects to execute for spawning the agent.
pub fn build_spawn_effects(
    agent_def: &AgentDef,
    pipeline: &Pipeline,
    pipeline_id: &PipelineId,
    agent_name: &str,
    input: &HashMap<String, String>,
    workspace_path: &Path,
) -> Result<Vec<Effect>, RuntimeError> {
    // Use workspace_path as project root for settings lookup
    // After the workspace refactor, the runbook's init step clones the project
    // into workspace_path, so settings are found there.
    let project_root = workspace_path.to_path_buf();

    tracing::debug!(
        pipeline_id = %pipeline_id,
        agent_name,
        workspace_path = %workspace_path.display(),
        project_root = %project_root.display(),
        "building spawn effects"
    );

    // Step 1: Build variables for prompt interpolation
    // Namespace pipeline vars under "var." prefix
    let mut prompt_vars: HashMap<String, String> = input
        .iter()
        .map(|(k, v)| (format!("var.{}", k), v.clone()))
        .collect();

    // Add system variables (not namespaced - these are always available)
    // These overwrite any bare input keys with the same name.
    // Generate a unique UUID for agent_id (used as --session-id for claude/claudeless)
    let agent_id = Uuid::new_v4().to_string();
    prompt_vars.insert("agent_id".to_string(), agent_id.clone());
    prompt_vars.insert("pipeline_id".to_string(), pipeline_id.to_string());
    prompt_vars.insert("name".to_string(), pipeline.name.clone());
    prompt_vars.insert(
        "workspace".to_string(),
        workspace_path.display().to_string(),
    );

    // Expose invoke.dir and local.* at top level
    for (key, val) in input.iter() {
        if key.starts_with("invoke.") || key.starts_with("local.") {
            prompt_vars.insert(key.clone(), val.clone());
        }
    }

    // Step 2: Render the agent's prompt template
    let rendered_prompt =
        agent_def
            .get_prompt(&prompt_vars)
            .map_err(|e| RuntimeError::PromptError {
                agent: agent_name.to_string(),
                message: e.to_string(),
            })?;

    // Step 3: Build variables for command interpolation
    // Include rendered prompt so ${prompt} in run command gets the full agent prompt
    // The prompt must be escaped for shell context since it will be embedded in
    // a command string that tmux runs via /bin/sh -c. Characters like backticks,
    // dollar signs, and backslashes have special meaning in shell double-quoted strings.
    let mut vars = prompt_vars.clone();
    vars.insert(
        "prompt".to_string(),
        escape_for_shell_double_quotes(&rendered_prompt),
    );

    // Prepare workspace directory (no longer writes settings)
    tracing::debug!(workspace_path = %workspace_path.display(), "preparing workspace");
    crate::workspace::prepare_for_agent(workspace_path).map_err(|e| {
        tracing::error!(error = %e, "workspace preparation failed");
        RuntimeError::Execute(ExecuteError::Shell(e.to_string()))
    })?;

    // Write prime script if agent has one
    let prime_path = if let Some(ref prime) = agent_def.prime {
        Some(
            crate::workspace::prepare_agent_prime(&agent_id, prime, &prompt_vars).map_err(|e| {
                tracing::error!(error = %e, "agent prime preparation failed");
                RuntimeError::Execute(ExecuteError::Shell(e.to_string()))
            })?,
        )
    } else {
        None
    };

    // Prepare settings file with hooks in OJ state directory
    let settings_path =
        crate::workspace::prepare_agent_settings(&agent_id, workspace_path, prime_path.as_deref())
            .map_err(|e| {
                tracing::error!(error = %e, "agent settings preparation failed");
                RuntimeError::Execute(ExecuteError::Shell(e.to_string()))
            })?;

    // Build base command and append session-id, settings, and prompt (if not inline)
    let base_command = agent_def.build_command(&vars);
    let command = if agent_def.run.contains("${prompt}") {
        // Prompt is inline in the command, add session-id and settings
        format!(
            "{} --session-id {} --settings {}",
            base_command,
            agent_id,
            settings_path.display()
        )
    } else {
        // Append prompt (may be empty if no prompt configured)
        format!(
            "{} --session-id {} --settings {} \"{}\"",
            base_command,
            agent_id,
            settings_path.display(),
            vars.get("prompt").unwrap_or(&String::new())
        )
    };
    let mut env = agent_def.build_env(&vars);

    // Pass OJ_NAMESPACE so nested `oj` calls inherit the project namespace
    if !pipeline.namespace.is_empty() {
        env.push(("OJ_NAMESPACE".to_string(), pipeline.namespace.clone()));
    }

    // Pass OJ_STATE_DIR so `oj` commands can connect to the right daemon socket
    if let Ok(state_dir) = std::env::var("OJ_STATE_DIR") {
        env.push(("OJ_STATE_DIR".to_string(), state_dir));
    }

    // Pass OJ_DAEMON_BINARY so agents can find the correct daemon binary when
    // running `oj` commands (prevents tmux environment inheritance issues)
    if let Ok(daemon_binary) = std::env::var("OJ_DAEMON_BINARY") {
        env.push(("OJ_DAEMON_BINARY".to_string(), daemon_binary));
    } else if let Ok(current_exe) = std::env::current_exe() {
        // The engine runs inside ojd, so current_exe is the daemon binary
        env.push((
            "OJ_DAEMON_BINARY".to_string(),
            current_exe.display().to_string(),
        ));
    }

    // Forward CLAUDE_CONFIG_DIR only if explicitly set — never fabricate a default.
    //
    // Claude Code stores auth (OAuth tokens) in $HOME/.claude.json and config
    // (settings, session logs, projects) in $CLAUDE_CONFIG_DIR/.claude.json,
    // defaulting CLAUDE_CONFIG_DIR to $HOME/.claude when unset.  These are two
    // different files at two different paths.
    //
    // If we fabricate CLAUDE_CONFIG_DIR=$HOME/.claude and pass it via tmux -e,
    // Claude Code looks for .claude.json at $HOME/.claude/.claude.json (the
    // config copy, which has no auth) instead of $HOME/.claude.json (the real
    // one with oauthAccount).  Result: agents get the onboarding/login flow
    // even though the user is already authenticated.
    //
    // The watcher (find_session_log) independently defaults to ~/.claude for
    // log discovery, matching Claude Code's own default — so both sides agree
    // without us needing to set anything.
    if !env.iter().any(|(k, _)| k == "CLAUDE_CONFIG_DIR") {
        if let Ok(claude_state) = std::env::var("CLAUDE_CONFIG_DIR") {
            env.push(("CLAUDE_CONFIG_DIR".to_string(), claude_state));
        }
    }

    // Forward CLAUDE_CODE_OAUTH_TOKEN so agents can authenticate in
    // headless/CI environments where interactive login isn't possible.
    if !env.iter().any(|(k, _)| k == "CLAUDE_CODE_OAUTH_TOKEN") {
        if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
            env.push(("CLAUDE_CODE_OAUTH_TOKEN".to_string(), token));
        } else if env.iter().any(|(k, _)| k == "CLAUDE_CONFIG_DIR") {
            tracing::warn!(
                "CLAUDE_CONFIG_DIR is set but CLAUDE_CODE_OAUTH_TOKEN is not; \
                 agents may fail to authenticate if no interactive login session exists"
            );
        }
    }

    // Determine effective working directory from agent cwd config
    // Default to workspace_path if no cwd specified
    let effective_cwd = agent_def.cwd.as_ref().map_or_else(
        || workspace_path.to_path_buf(),
        |cwd_template| {
            let cwd_str = oj_runbook::interpolate(cwd_template, &vars);
            if Path::new(&cwd_str).is_absolute() {
                PathBuf::from(cwd_str)
            } else {
                workspace_path.join(cwd_str)
            }
        },
    );

    tracing::info!(
        pipeline_id = %pipeline_id,
        agent_name,
        command,
        effective_cwd = ?effective_cwd,
        "spawn effects prepared"
    );

    Ok(vec![
        Effect::SpawnAgent {
            agent_id: AgentId::new(agent_id),
            agent_name: agent_name.to_string(),
            pipeline_id: pipeline_id.clone(),
            workspace_path: workspace_path.to_path_buf(),
            input: vars,
            command,
            env,
            cwd: Some(effective_cwd),
        },
        // Start liveness monitoring timer
        Effect::SetTimer {
            id: TimerId::liveness(pipeline_id),
            duration: LIVENESS_INTERVAL,
        },
    ])
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
