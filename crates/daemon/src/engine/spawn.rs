// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent spawning functionality

use crate::engine::error::RuntimeError;
use crate::engine::executor::ExecuteError;
use oj_core::{AgentId, CrewId, Effect, Job, JobId, OwnerId};
use oj_runbook::AgentDef;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

/// Liveness check interval (30 seconds)
pub const LIVENESS_INTERVAL: Duration = Duration::from_secs(30);

/// Context for spawning an agent, abstracting over jobs and standalone runs.
pub struct SpawnCtx<'a> {
    /// Owner of this agent (job or crew)
    pub owner: OwnerId,
    /// Display name (job name or command name)
    pub name: &'a str,
    /// Namespace for scoping
    pub project: &'a str,
}

impl<'a> SpawnCtx<'a> {
    /// Create a SpawnCtx from a Job.
    pub fn from_job(job: &'a Job, job_id: &JobId) -> Self {
        Self { owner: OwnerId::Job(job_id.clone()), name: &job.name, project: &job.project }
    }

    /// Create a SpawnCtx for a crew.
    pub fn from_crew(crew_id: &CrewId, name: &'a str, project: &'a str) -> Self {
        Self { owner: OwnerId::Crew(crew_id.clone()), name, project }
    }
}

/// Spawn an agent for a job or standalone run.
///
/// Returns the effects to execute for spawning the agent.
/// When `resume` is true, the agent is spawned with coop's `--resume` flag
/// to preserve conversation history from a previous run.
pub fn build_spawn_effects(
    agent_def: &AgentDef,
    ctx: &SpawnCtx<'_>,
    agent_name: &str,
    input: &HashMap<String, String>,
    workspace_path: &Path,
    state_dir: &Path,
    resume: bool,
) -> Result<Vec<Effect>, RuntimeError> {
    // Step 1: Build variables for prompt interpolation
    let mut prompt_vars = crate::engine::vars::namespace_vars(input);

    // Add system variables (not namespaced - these are always available)
    // These overwrite any bare input keys with the same name.
    // Always generate a fresh UUID for agent_id (oddjobs tracking concept,
    // decoupled from Claude's internal session/conversation ID).
    let agent_id = Uuid::new_v4().to_string();
    prompt_vars.insert("agent_id".to_string(), agent_id.clone());
    // Insert owner-specific ID: job_id for jobs, crew_id for standalone runs
    match &ctx.owner {
        OwnerId::Job(job_id) => {
            prompt_vars.insert("job_id".to_string(), job_id.to_string());
        }
        OwnerId::Crew(run_id) => {
            prompt_vars.insert("crew_id".to_string(), run_id.to_string());
        }
    }
    prompt_vars.insert("name".to_string(), ctx.name.to_string());
    prompt_vars.insert("workspace".to_string(), workspace_path.display().to_string());

    // Expose invoke.*, local.*, and source.* at top level
    for (key, val) in input.iter() {
        if key.starts_with("invoke.") || key.starts_with("local.") || key.starts_with("source.") {
            prompt_vars.insert(key.clone(), val.clone());
        }
    }

    // Step 2: Render the agent's prompt template
    let rendered_prompt = agent_def.get_prompt(&prompt_vars).map_err(|e| {
        RuntimeError::PromptError { agent: agent_name.to_string(), message: e.to_string() }
    })?;

    // Step 3: Build variables for command interpolation
    // Include rendered prompt so ${prompt} in run command gets the full agent prompt
    // The prompt must be escaped for shell context since it will be embedded in
    // a command string that coop runs via bash -c. Characters like backticks,
    // dollar signs, and backslashes have special meaning in shell double-quoted strings.
    let mut vars = prompt_vars.clone();
    vars.insert("prompt".to_string(), oj_runbook::escape_for_shell(&rendered_prompt));

    // Write agent-config file with settings, stop config, and start config for primes.
    // Coop reads this file and handles passing --settings to claude.
    //
    // Resolve on_idle action for stop config. The runbook default is `escalate`;
    // for job-owned agents we override to `done` (complete on stop) unless
    // explicitly configured.
    let effective_action = match (&agent_def.on_idle, &ctx.owner) {
        (None, OwnerId::Job(_)) => "done",
        (None, _) => "escalate",
        (Some(cfg), _) => cfg.action().as_str(),
    };
    let on_idle_message = agent_def.on_idle.as_ref().and_then(|c| c.message());
    crate::engine::agent_setup::write_agent_config_file(
        &agent_id,
        workspace_path,
        agent_def.prime.as_ref(),
        &prompt_vars,
        effective_action,
        on_idle_message,
        state_dir,
    )
    .map_err(|e| {
        tracing::error!(error = %e, "agent config file write failed");
        RuntimeError::Execute(ExecuteError::Shell(e.to_string()))
    })?;

    // Build base command and append prompt or resume message.
    // Settings are passed via coop's --agent-config, not via claude's --settings.
    // Trim trailing whitespace (including newlines from heredocs) so appended args stay on same line.
    let base_command = agent_def.build_command(&vars).trim_end().to_string();
    let command = if resume {
        // Resume mode: coop handles --resume via session discovery.
        // Prefer resume_message (append mode), fall back to prompt (original prompt).
        // The prompt fallback ensures `-p` mode agents get their prompt on retry.
        let resume_msg = input
            .get("resume_message")
            .or_else(|| input.get("prompt"))
            .cloned()
            .unwrap_or_default();
        if resume_msg.is_empty() {
            base_command
        } else {
            format!("{} \"{}\"", base_command, oj_runbook::escape_for_shell(&resume_msg))
        }
    } else if agent_def.run.contains("${prompt}") {
        // Prompt is inline in the command
        base_command
    } else {
        // Append prompt (may be empty if no prompt configured)
        format!("{} \"{}\"", base_command, vars.get("prompt").unwrap_or(&String::new()))
    };
    let mut env = agent_def.build_env(&vars);
    let mut unset_env: Vec<String> = Vec::new();

    // Pass OJ_PROJECT so nested `oj` calls inherit the project project
    if !ctx.project.is_empty() {
        env.push(("OJ_PROJECT".to_string(), ctx.project.to_string()));
    }

    // Always pass OJ_STATE_DIR so `oj` commands (including hooks) connect to
    // the right daemon socket. Use the state_dir parameter — the daemon's actual
    // state directory — rather than reading from the environment. The daemon may
    // have resolved its state_dir via XDG_STATE_HOME or $HOME fallback without
    // OJ_STATE_DIR being set, so the env var alone is unreliable.
    env.push(("OJ_STATE_DIR".to_string(), state_dir.to_string_lossy().into_owned()));

    // Pass OJ_DAEMON_BINARY so agents can find the correct daemon binary when
    // running `oj` commands (prevents environment inheritance issues)
    if let Ok(daemon_binary) = std::env::var("OJ_DAEMON_BINARY") {
        env.push(("OJ_DAEMON_BINARY".to_string(), daemon_binary));
    } else if let Ok(current_exe) = std::env::current_exe() {
        // The engine runs inside ojd, so current_exe is the daemon binary
        env.push(("OJ_DAEMON_BINARY".to_string(), current_exe.display().to_string()));
    }

    // Forward CLAUDE_CONFIG_DIR only if explicitly set — never fabricate a default.
    //
    // Claude Code stores auth (OAuth tokens) in $HOME/.claude.json and config
    // (settings, session logs, projects) in $CLAUDE_CONFIG_DIR/.claude.json,
    // defaulting CLAUDE_CONFIG_DIR to $HOME/.claude when unset.  These are two
    // different files at two different paths.
    //
    // If we fabricate CLAUDE_CONFIG_DIR=$HOME/.claude and pass it to coop,
    // Claude Code looks for .claude.json at $HOME/.claude/.claude.json (the
    // config copy, which has no auth) instead of $HOME/.claude.json (the real
    // one with oauthAccount).  Result: agents get the onboarding/login flow
    // even though the user is already authenticated.
    if !env.iter().any(|(k, _)| k == "CLAUDE_CONFIG_DIR") {
        if let Ok(claude_state) = std::env::var("CLAUDE_CONFIG_DIR") {
            env.push(("CLAUDE_CONFIG_DIR".to_string(), claude_state));
        } else {
            // Prevent stale CLAUDE_CONFIG_DIR from parent environment
            // causing agents to look for auth at the wrong path.
            unset_env.push("CLAUDE_CONFIG_DIR".to_string());
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
        owner = %ctx.owner,
        agent_name,
        command,
        effective_cwd = ?effective_cwd,
        "spawn effects prepared"
    );

    // Resolve container config: agent-level takes priority, then falls back to
    // the container config passed in by the caller (from the job definition).
    let container = agent_def.container.as_ref().map(|c| oj_core::ContainerConfig::new(&c.image));

    Ok(vec![Effect::SpawnAgent {
        agent_id: AgentId::new(agent_id),
        agent_name: agent_name.to_string(),
        owner: ctx.owner.clone(),
        workspace_path: workspace_path.to_path_buf(),
        input: vars,
        command,
        env,
        unset_env,
        cwd: Some(effective_cwd),
        resume,
        container,
    }])
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
