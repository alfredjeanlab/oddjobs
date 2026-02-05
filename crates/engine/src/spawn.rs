// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent spawning functionality

use crate::error::RuntimeError;
use crate::executor::ExecuteError;
use oj_core::{AgentId, AgentRunId, Effect, Job, JobId, ShortId, TimerId};
use oj_runbook::{AgentDef, StopAction};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

/// Liveness check interval (30 seconds)
pub const LIVENESS_INTERVAL: Duration = Duration::from_secs(30);

/// Context for spawning an agent, abstracting over jobs and standalone runs.
pub struct SpawnContext<'a> {
    /// Job ID (set for job agents, empty for standalone)
    pub job_id: &'a JobId,
    /// Optional agent run ID (set for standalone agents)
    pub agent_run_id: Option<&'a AgentRunId>,
    /// Display name (job name or command name)
    pub name: &'a str,
    /// Namespace for scoping
    pub namespace: &'a str,
}

impl<'a> SpawnContext<'a> {
    /// Create a SpawnContext from a Job.
    pub fn from_job(job: &'a Job, job_id: &'a JobId) -> Self {
        Self {
            job_id,
            agent_run_id: None,
            name: &job.name,
            namespace: &job.namespace,
        }
    }
}

/// Spawn an agent for a job or standalone run.
///
/// Returns the effects to execute for spawning the agent.
/// When `resume_session_id` is `Some`, the agent is spawned with `--resume <id>`
/// to preserve conversation history from a previous run.
pub fn build_spawn_effects(
    agent_def: &AgentDef,
    ctx: &SpawnContext<'_>,
    agent_name: &str,
    input: &HashMap<String, String>,
    workspace_path: &Path,
    state_dir: &Path,
    resume_session_id: Option<&str>,
) -> Result<Vec<Effect>, RuntimeError> {
    // Use workspace_path as project root for settings lookup
    // After the workspace refactor, the runbook's init step clones the project
    // into workspace_path, so settings are found there.
    let project_root = workspace_path.to_path_buf();

    tracing::debug!(
        owner_id = %ctx.job_id,
        agent_name,
        workspace_path = %workspace_path.display(),
        project_root = %project_root.display(),
        "building spawn effects"
    );

    // Step 1: Build variables for prompt interpolation
    // Namespace job vars under "var." prefix
    let mut prompt_vars: HashMap<String, String> = input
        .iter()
        .map(|(k, v)| (format!("var.{}", k), v.clone()))
        .collect();

    // Add system variables (not namespaced - these are always available)
    // These overwrite any bare input keys with the same name.
    // Generate a unique UUID for agent_id (used as --session-id for claude/claudeless)
    let agent_id = Uuid::new_v4().to_string();
    prompt_vars.insert("agent_id".to_string(), agent_id.clone());
    prompt_vars.insert("job_id".to_string(), ctx.job_id.to_string());
    prompt_vars.insert("name".to_string(), ctx.name.to_string());
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
        oj_runbook::escape_for_shell(&rendered_prompt),
    );

    // Prepare workspace directory (no longer writes settings)
    tracing::debug!(workspace_path = %workspace_path.display(), "preparing workspace");
    crate::workspace::prepare_for_agent(workspace_path).map_err(|e| {
        tracing::error!(error = %e, "workspace preparation failed");
        RuntimeError::Execute(ExecuteError::Shell(e.to_string()))
    })?;

    // Write prime script(s) if agent has prime config
    let prime_paths = if let Some(ref prime) = agent_def.prime {
        crate::workspace::prepare_agent_prime(&agent_id, prime, &prompt_vars, state_dir).map_err(
            |e| {
                tracing::error!(error = %e, "agent prime preparation failed");
                RuntimeError::Execute(ExecuteError::Shell(e.to_string()))
            },
        )?
    } else {
        HashMap::new()
    };

    // Prepare settings file with hooks in OJ state directory
    let settings_path = crate::workspace::prepare_agent_settings(
        &agent_id,
        workspace_path,
        &prime_paths,
        state_dir,
    )
    .map_err(|e| {
        tracing::error!(error = %e, "agent settings preparation failed");
        RuntimeError::Execute(ExecuteError::Shell(e.to_string()))
    })?;

    // Write on_stop config: resolve from agent def or context-dependent default
    let is_standalone = ctx.agent_run_id.is_some();
    let on_stop_action = agent_def
        .on_stop
        .as_ref()
        .map(|c| c.action())
        .cloned()
        .unwrap_or(if is_standalone {
            StopAction::Escalate
        } else {
            StopAction::Signal
        });
    let on_stop_str = match on_stop_action {
        StopAction::Signal => "signal",
        StopAction::Idle => "idle",
        StopAction::Escalate => "escalate",
    };
    crate::workspace::write_agent_config(&agent_id, on_stop_str, state_dir).map_err(|e| {
        tracing::error!(error = %e, "agent config write failed");
        RuntimeError::Execute(ExecuteError::Shell(e.to_string()))
    })?;

    // Build base command and append session-id, settings, and prompt (if not inline)
    let base_command = agent_def.build_command(&vars);
    let command = if let Some(resume_id) = resume_session_id {
        // Resume mode: use --resume instead of passing prompt
        let resume_msg = input.get("resume_message").cloned().unwrap_or_default();
        if resume_msg.is_empty() {
            format!(
                "{} --resume {} --session-id {} --settings {}",
                base_command,
                resume_id,
                agent_id,
                settings_path.display()
            )
        } else {
            format!(
                "{} --resume {} --session-id {} --settings {} \"{}\"",
                base_command,
                resume_id,
                agent_id,
                settings_path.display(),
                oj_runbook::escape_for_shell(&resume_msg)
            )
        }
    } else if agent_def.run.contains("${prompt}") {
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
    if !ctx.namespace.is_empty() {
        env.push(("OJ_NAMESPACE".to_string(), ctx.namespace.to_string()));
    }

    // Always pass OJ_STATE_DIR so `oj` commands (including hooks) connect to
    // the right daemon socket. Use the state_dir parameter — the daemon's actual
    // state directory — rather than reading from the environment. The daemon may
    // have resolved its state_dir via XDG_STATE_HOME or $HOME fallback without
    // OJ_STATE_DIR being set, so the env var alone is unreliable.
    env.push((
        "OJ_STATE_DIR".to_string(),
        state_dir.to_string_lossy().into_owned(),
    ));

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
        owner_id = %ctx.job_id,
        agent_name,
        command,
        effective_cwd = ?effective_cwd,
        "spawn effects prepared"
    );

    // Build session config with defaults
    let session_config = {
        let mut config: HashMap<String, serde_json::Value> = HashMap::new();
        if let Some(tmux_cfg) = agent_def.session.get("tmux") {
            if let Ok(val) = serde_json::to_value(tmux_cfg) {
                config.insert("tmux".to_string(), val);
            }
        }
        // Always ensure tmux has default status bars (even without explicit session block)
        let tmux_value = config
            .entry("tmux".to_string())
            .or_insert_with(|| serde_json::json!({}));
        if let serde_json::Value::Object(ref mut map) = tmux_value {
            let status = map.entry("status").or_insert_with(|| serde_json::json!({}));
            if let serde_json::Value::Object(ref mut status_map) = status {
                let short_id = agent_id.short(8);
                status_map.entry("left").or_insert_with(|| {
                    serde_json::json!(format!("{} {}/{}", ctx.namespace, ctx.name, agent_name))
                });
                status_map
                    .entry("right")
                    .or_insert_with(|| serde_json::json!(short_id));
            }
        }
        config
    };

    // Build liveness timer keyed to the right owner
    let liveness_timer_id = if let Some(ar_id) = ctx.agent_run_id {
        TimerId::liveness_agent_run(ar_id)
    } else {
        TimerId::liveness(ctx.job_id)
    };

    Ok(vec![
        Effect::SpawnAgent {
            agent_id: AgentId::new(agent_id),
            agent_name: agent_name.to_string(),
            job_id: ctx.job_id.clone(),
            agent_run_id: ctx.agent_run_id.cloned(),
            workspace_path: workspace_path.to_path_buf(),
            input: vars,
            command,
            env,
            cwd: Some(effective_cwd),
            session_config,
        },
        // Start liveness monitoring timer
        Effect::SetTimer {
            id: liveness_timer_id,
            duration: LIVENESS_INTERVAL,
        },
    ])
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
