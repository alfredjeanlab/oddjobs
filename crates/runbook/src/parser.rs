// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runbook parsing (TOML, HCL, and JSON)

use crate::import::{ConstDef, ImportDef};
use crate::validate::{
    sorted_keys, sorted_names, validate_agent_command, validate_command_template_refs,
    validate_duration_str, validate_shell_command, validate_template_namespaces,
};
use crate::{
    ActionTrigger, AgentDef, ArgSpecError, CommandDef, CronDef, JobDef, PrimeDef, QueueDef,
    QueueType, RunDirective, WorkerDef,
};
use oj_shell as shell;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Runbook file format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Toml,
    Hcl,
    Json,
}

/// Errors that can occur during runbook parsing
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("HCL parse error: {0}")]
    Hcl(#[from] hcl::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid format for {location}: {message}")]
    InvalidFormat { location: String, message: String },

    #[error(
        "invalid shell command in {location}:\n{}",
        shell_diagnostic(inner, source_text)
    )]
    ShellError {
        location: String,
        inner: Box<shell::ParseError>,
        source_text: String,
    },

    #[error(
        "invalid shell command in {location}:\n{}",
        validation_diagnostic(inner, source_text)
    )]
    ShellValidation {
        location: String,
        inner: Box<shell::ValidationError>,
        source_text: String,
    },

    #[error("invalid argument spec: {0}")]
    ArgSpec(#[from] ArgSpecError),
}

/// A parsed runbook
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runbook {
    #[serde(default, alias = "import")]
    pub imports: HashMap<String, ImportDef>,
    #[serde(default, alias = "const")]
    pub consts: HashMap<String, ConstDef>,
    #[serde(default, alias = "command")]
    pub commands: HashMap<String, CommandDef>,
    #[serde(default, alias = "job")]
    pub jobs: HashMap<String, JobDef>,
    #[serde(default, alias = "agent")]
    pub agents: HashMap<String, AgentDef>,
    #[serde(default, alias = "queue")]
    pub queues: HashMap<String, QueueDef>,
    #[serde(default, alias = "worker")]
    pub workers: HashMap<String, WorkerDef>,
    #[serde(default, alias = "cron")]
    pub crons: HashMap<String, CronDef>,
}

impl Runbook {
    /// Get a command definition by name
    pub fn get_command(&self, name: &str) -> Option<&CommandDef> {
        self.commands.get(name)
    }

    /// Get a job definition by name
    pub fn get_job(&self, name: &str) -> Option<&JobDef> {
        self.jobs.get(name)
    }

    /// Get an agent definition by name
    pub fn get_agent(&self, name: &str) -> Option<&AgentDef> {
        self.agents.get(name)
    }

    /// Get a queue definition by name
    pub fn get_queue(&self, name: &str) -> Option<&QueueDef> {
        self.queues.get(name)
    }

    /// Get a worker definition by name
    pub fn get_worker(&self, name: &str) -> Option<&WorkerDef> {
        self.workers.get(name)
    }

    /// Get a cron definition by name
    pub fn get_cron(&self, name: &str) -> Option<&CronDef> {
        self.crons.get(name)
    }
}

/// Format a shell parse error as a diagnostic with source snippet.
fn shell_diagnostic(err: &shell::ParseError, source: &str) -> String {
    err.diagnostic(source).unwrap_or_else(|| err.to_string())
}

/// Format a shell validation error as a diagnostic with source snippet.
fn validation_diagnostic(err: &shell::ValidationError, source: &str) -> String {
    err.diagnostic(source)
}

/// Parse a runbook from TOML content (convenience wrapper)
pub fn parse_runbook(content: &str) -> Result<Runbook, ParseError> {
    parse_runbook_with_format(content, Format::Toml)
}

/// Parse a runbook from the given content in the specified format
pub fn parse_runbook_with_format(content: &str, format: Format) -> Result<Runbook, ParseError> {
    parse_runbook_inner(content, format, true)
}

/// Parse a runbook without cross-reference validation.
///
/// Used by the import system: individual files are parsed without cross-refs,
/// then imports are resolved and merged, and cross-refs are validated on the
/// merged result.
pub(crate) fn parse_runbook_no_xref(content: &str, format: Format) -> Result<Runbook, ParseError> {
    // Same as parse_runbook_with_format but stops before cross-ref validation.
    // We call the main function's logic up to step 11, then skip step 12.
    parse_runbook_inner(content, format, false)
}

/// Inner parse function with optional cross-ref validation.
fn parse_runbook_inner(
    content: &str,
    format: Format,
    validate_refs: bool,
) -> Result<Runbook, ParseError> {
    // 1. Serde does the heavy lifting
    let mut runbook: Runbook = match format {
        Format::Toml => toml::from_str(content)?,
        Format::Hcl => hcl::from_str(content)?,
        Format::Json => serde_json::from_str(content)?,
    };

    // 2. Name fixup — inject map keys into .name fields
    for (name, cmd) in &mut runbook.commands {
        cmd.name = name.clone();
    }
    for (name, job) in &mut runbook.jobs {
        job.kind = name.clone();
    }
    for (name, agent) in &mut runbook.agents {
        agent.name = name.clone();
    }
    for (name, queue) in &mut runbook.queues {
        queue.name = name.clone();
    }
    for (name, worker) in &mut runbook.workers {
        worker.name = name.clone();
    }
    for (name, cron) in &mut runbook.crons {
        cron.name = name.clone();
    }

    // 3. Validation — step names must not be empty
    for (job_name, job) in &runbook.jobs {
        for (i, step) in job.steps.iter().enumerate() {
            if step.name.is_empty() {
                return Err(ParseError::InvalidFormat {
                    location: format!("job.{}.step[{}]", job_name, i),
                    message: "step name is required".to_string(),
                });
            }
        }
    }

    // 4. Validation — shell command syntax and agent command checks
    for (name, cmd) in &runbook.commands {
        if let RunDirective::Shell(ref shell_cmd) = cmd.run {
            let location = format!("command.{}.run", name);
            validate_shell_command(shell_cmd, &location)?;
            validate_command_template_refs(shell_cmd, &location)?;
        }
    }

    for (job_name, job) in &runbook.jobs {
        for (i, step) in job.steps.iter().enumerate() {
            let step_location = format!("job.{}.step[{}]({}).run", job_name, i, step.name);
            if let RunDirective::Shell(ref shell_cmd) = step.run {
                validate_shell_command(shell_cmd, &step_location)?;
                validate_template_namespaces(shell_cmd, &step_location)?;
            }
        }
        // Validate job local variable templates
        for (local_name, local_value) in &job.locals {
            let local_location = format!("job.{}.locals.{}", job_name, local_name);
            validate_template_namespaces(local_value, &local_location)?;
        }
    }

    for (name, agent) in &runbook.agents {
        let has_prompt = agent.prompt.is_some() || agent.prompt_file.is_some();

        if !agent.run.is_empty() {
            let run_location = format!("agent.{}.run", name);
            validate_shell_command(&agent.run, &run_location)?;
            validate_agent_command(&agent.run, &run_location, has_prompt)?;
        }
        // Validate agent prompt templates
        if let Some(ref prompt) = agent.prompt {
            validate_template_namespaces(prompt, &format!("agent.{}.prompt", name))?;
        }

        if let Some(ref prime) = agent.prime {
            match prime {
                PrimeDef::Commands(cmds) => {
                    for (i, cmd) in cmds.iter().enumerate() {
                        validate_shell_command(cmd, &format!("agent.{}.prime[{}]", name, i))?;
                    }
                }
                PrimeDef::PerSource(map) => {
                    for (source, def) in map {
                        if !crate::agent::VALID_PRIME_SOURCES.contains(&source.as_str()) {
                            return Err(ParseError::InvalidFormat {
                                location: format!("agent.{}.prime", name),
                                message: format!(
                                    "unknown prime source '{}'; valid sources: {}",
                                    source,
                                    crate::agent::VALID_PRIME_SOURCES.join(", ")
                                ),
                            });
                        }
                        if let PrimeDef::Commands(cmds) = def {
                            for (i, cmd) in cmds.iter().enumerate() {
                                validate_shell_command(
                                    cmd,
                                    &format!("agent.{}.prime.{}[{}]", name, source, i),
                                )?;
                            }
                        }
                    }
                }
                PrimeDef::Script(_) => {}
            }
        }
    }

    // 5. Validate queue fields by type
    for (name, queue) in &runbook.queues {
        match queue.queue_type {
            QueueType::External => {
                let list = queue
                    .list
                    .as_deref()
                    .ok_or_else(|| ParseError::InvalidFormat {
                        location: format!("queue.{}", name),
                        message: "external queue requires 'list' field".to_string(),
                    })?;
                let take = queue
                    .take
                    .as_deref()
                    .ok_or_else(|| ParseError::InvalidFormat {
                        location: format!("queue.{}", name),
                        message: "external queue requires 'take' field".to_string(),
                    })?;
                validate_shell_command(list, &format!("queue.{}.list", name))?;
                validate_shell_command(take, &format!("queue.{}.take", name))?;
                if queue.retry.is_some() {
                    return Err(ParseError::InvalidFormat {
                        location: format!("queue.{}", name),
                        message: "external queue must not have 'retry' field".to_string(),
                    });
                }
                if let Some(ref poll) = queue.poll {
                    if let Err(e) = validate_duration_str(poll) {
                        return Err(ParseError::InvalidFormat {
                            location: format!("queue.{}.poll", name),
                            message: e,
                        });
                    }
                }
            }
            QueueType::Persisted => {
                if queue.vars.is_empty() {
                    return Err(ParseError::InvalidFormat {
                        location: format!("queue.{}", name),
                        message: "persisted queue requires 'vars' field".to_string(),
                    });
                }
                if queue.list.is_some() {
                    return Err(ParseError::InvalidFormat {
                        location: format!("queue.{}", name),
                        message: "persisted queue must not have 'list' field".to_string(),
                    });
                }
                if queue.take.is_some() {
                    return Err(ParseError::InvalidFormat {
                        location: format!("queue.{}", name),
                        message: "persisted queue must not have 'take' field".to_string(),
                    });
                }
                if queue.poll.is_some() {
                    return Err(ParseError::InvalidFormat {
                        location: format!("queue.{}", name),
                        message: "persisted queue must not have 'poll' field".to_string(),
                    });
                }
                if let Some(ref retry) = queue.retry {
                    if let Err(e) = validate_duration_str(&retry.cooldown) {
                        return Err(ParseError::InvalidFormat {
                            location: format!("queue.{}.retry.cooldown", name),
                            message: e,
                        });
                    }
                }
            }
        }
    }

    // 6.5. Validate cron interval syntax
    for (name, cron) in &runbook.crons {
        if let Err(e) = validate_duration_str(&cron.interval) {
            return Err(ParseError::InvalidFormat {
                location: format!("cron.{}.interval", name),
                message: e,
            });
        }
    }

    // 6.6. Validate agent max_concurrency
    for (name, agent) in &runbook.agents {
        if let Some(max) = agent.max_concurrency {
            if max == 0 {
                return Err(ParseError::InvalidFormat {
                    location: format!("agent.{}.max_concurrency", name),
                    message: "max_concurrency must be >= 1".to_string(),
                });
            }
        }
    }

    // 6.7. Validate tmux session config colors
    for (agent_name, agent) in &runbook.agents {
        if let Some(tmux_config) = agent.session.get("tmux") {
            if let Some(ref color) = tmux_config.color {
                if !crate::agent::VALID_SESSION_COLORS.contains(&color.as_str()) {
                    return Err(ParseError::InvalidFormat {
                        location: format!("agent.{}.session.tmux.color", agent_name),
                        message: format!(
                            "unknown color '{}'; valid colors: {}",
                            color,
                            crate::agent::VALID_SESSION_COLORS.join(", ")
                        ),
                    });
                }
            }
        }
    }

    // 7. Validate action-trigger compatibility
    for (agent_name, agent) in &runbook.agents {
        // Validate on_idle action
        let idle_action = agent.on_idle.action();
        if !idle_action.is_valid_for_trigger(ActionTrigger::OnIdle) {
            return Err(ParseError::InvalidFormat {
                location: format!("agent.{}.on_idle", agent_name),
                message: format!(
                    "action '{}' is not valid for on_idle: {}",
                    idle_action.as_str(),
                    idle_action.invalid_reason(ActionTrigger::OnIdle)
                ),
            });
        }

        // Validate on_dead action
        let dead_action = agent.on_dead.action();
        if !dead_action.is_valid_for_trigger(ActionTrigger::OnDead) {
            return Err(ParseError::InvalidFormat {
                location: format!("agent.{}.on_dead", agent_name),
                message: format!(
                    "action '{}' is not valid for on_dead: {}",
                    dead_action.as_str(),
                    dead_action.invalid_reason(ActionTrigger::OnDead)
                ),
            });
        }

        // Validate on_prompt action
        let prompt_action = agent.on_prompt.action();
        if !prompt_action.is_valid_for_trigger(ActionTrigger::OnPrompt) {
            return Err(ParseError::InvalidFormat {
                location: format!("agent.{}.on_prompt", agent_name),
                message: format!(
                    "action '{}' is not valid for on_prompt: {}",
                    prompt_action.as_str(),
                    prompt_action.invalid_reason(ActionTrigger::OnPrompt)
                ),
            });
        }

        // Validate on_error action(s)
        for error_action in agent.on_error.all_actions() {
            if !error_action.is_valid_for_trigger(ActionTrigger::OnError) {
                return Err(ParseError::InvalidFormat {
                    location: format!("agent.{}.on_error", agent_name),
                    message: format!(
                        "action '{}' is not valid for on_error: {}",
                        error_action.as_str(),
                        error_action.invalid_reason(ActionTrigger::OnError)
                    ),
                });
            }
        }
    }

    // 8. Detect duplicate step names within jobs
    for (job_name, job) in &runbook.jobs {
        let mut seen = HashSet::new();
        for (i, step) in job.steps.iter().enumerate() {
            if !seen.insert(step.name.as_str()) {
                return Err(ParseError::InvalidFormat {
                    location: format!("job.{}.step[{}]({})", job_name, i, step.name),
                    message: format!("duplicate step name '{}'", step.name),
                });
            }
        }
    }

    // 9. Validate step transition references
    for (job_name, job) in &runbook.jobs {
        let step_names: HashSet<&str> = job.steps.iter().map(|s| s.name.as_str()).collect();

        // Check job-level transitions
        for (field, transition) in [
            ("on_done", &job.on_done),
            ("on_fail", &job.on_fail),
            ("on_cancel", &job.on_cancel),
        ] {
            if let Some(t) = transition {
                if !step_names.contains(t.step_name()) {
                    return Err(ParseError::InvalidFormat {
                        location: format!("job.{}.{}", job_name, field),
                        message: format!(
                            "references unknown step '{}'; available steps: {}",
                            t.step_name(),
                            sorted_names(&step_names),
                        ),
                    });
                }
            }
        }

        // Check step-level transitions
        for (i, step) in job.steps.iter().enumerate() {
            for (field, transition) in [
                ("on_done", &step.on_done),
                ("on_fail", &step.on_fail),
                ("on_cancel", &step.on_cancel),
            ] {
                if let Some(t) = transition {
                    if !step_names.contains(t.step_name()) {
                        return Err(ParseError::InvalidFormat {
                            location: format!(
                                "job.{}.step[{}]({}).{}",
                                job_name, i, step.name, field
                            ),
                            message: format!(
                                "references unknown step '{}'; available steps: {}",
                                t.step_name(),
                                sorted_names(&step_names),
                            ),
                        });
                    }
                }
            }
        }
    }

    // 11. Warn on unreachable steps
    let mut sorted_jobs: Vec<_> = runbook.jobs.iter().collect();
    sorted_jobs.sort_by_key(|(name, _)| *name);
    for (job_name, job) in sorted_jobs {
        if job.steps.len() <= 1 {
            continue;
        }
        let mut referenced: HashSet<&str> = HashSet::new();
        for t in [&job.on_done, &job.on_fail, &job.on_cancel]
            .into_iter()
            .flatten()
        {
            referenced.insert(t.step_name());
        }
        for step in &job.steps {
            for t in [&step.on_done, &step.on_fail, &step.on_cancel]
                .into_iter()
                .flatten()
            {
                referenced.insert(t.step_name());
            }
        }
        for step in job.steps.iter().skip(1) {
            if !referenced.contains(step.name.as_str()) {
                return Err(ParseError::InvalidFormat {
                    location: format!("job.{}.step.{}", job_name, step.name),
                    message: format!(
                        "step '{}' is unreachable \
                         (not referenced by any on_done/on_fail/on_cancel)",
                        step.name
                    ),
                });
            }
        }
    }

    if validate_refs {
        validate_cross_refs(&runbook)?;
    }

    Ok(runbook)
}

/// Validate cross-references between entities in a runbook.
///
/// Checks that:
/// - Workers reference existing queues and jobs
/// - Crons reference existing jobs or agents
/// - Steps and commands reference existing agents and jobs
pub(crate) fn validate_cross_refs(runbook: &Runbook) -> Result<(), ParseError> {
    // Worker cross-references
    for (name, worker) in &runbook.workers {
        if !runbook.queues.contains_key(&worker.source.queue) {
            return Err(ParseError::InvalidFormat {
                location: format!("worker.{}.source.queue", name),
                message: format!(
                    "references unknown queue '{}'; available queues: {}",
                    worker.source.queue,
                    runbook
                        .queues
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            });
        }
        if !runbook.jobs.contains_key(&worker.handler.job) {
            return Err(ParseError::InvalidFormat {
                location: format!("worker.{}.handler.job", name),
                message: format!(
                    "references unknown job '{}'; available jobs: {}",
                    worker.handler.job,
                    runbook.jobs.keys().cloned().collect::<Vec<_>>().join(", "),
                ),
            });
        }
    }

    // Cron cross-references
    for (name, cron) in &runbook.crons {
        match &cron.run {
            RunDirective::Job { job } => {
                if !runbook.jobs.contains_key(job.as_str()) {
                    return Err(ParseError::InvalidFormat {
                        location: format!("cron.{}.run", name),
                        message: format!(
                            "references unknown job '{}'; available jobs: {}",
                            job,
                            sorted_keys(&runbook.jobs),
                        ),
                    });
                }
            }
            RunDirective::Agent { agent, .. } => {
                if !runbook.agents.contains_key(agent.as_str()) {
                    return Err(ParseError::InvalidFormat {
                        location: format!("cron.{}.run", name),
                        message: format!(
                            "references unknown agent '{}'; available agents: {}",
                            agent,
                            sorted_keys(&runbook.agents),
                        ),
                    });
                }
            }
            RunDirective::Shell(_) => {
                return Err(ParseError::InvalidFormat {
                    location: format!("cron.{}.run", name),
                    message: "cron run must reference a job or agent".to_string(),
                });
            }
        }
    }

    // Step and command cross-references
    for (job_name, job) in &runbook.jobs {
        for (i, step) in job.steps.iter().enumerate() {
            if let Some(agent_name) = step.run.agent_name() {
                if !runbook.agents.contains_key(agent_name) {
                    return Err(ParseError::InvalidFormat {
                        location: format!("job.{}.step[{}]({}).run", job_name, i, step.name),
                        message: format!(
                            "references unknown agent '{}'; available agents: {}",
                            agent_name,
                            sorted_keys(&runbook.agents),
                        ),
                    });
                }
            }
            if let Some(pl_name) = step.run.job_name() {
                if !runbook.jobs.contains_key(pl_name) {
                    return Err(ParseError::InvalidFormat {
                        location: format!("job.{}.step[{}]({}).run", job_name, i, step.name),
                        message: format!(
                            "references unknown job '{}'; available jobs: {}",
                            pl_name,
                            sorted_keys(&runbook.jobs),
                        ),
                    });
                }
            }
        }
    }

    for (cmd_name, cmd) in &runbook.commands {
        if let Some(agent_name) = cmd.run.agent_name() {
            if !runbook.agents.contains_key(agent_name) {
                return Err(ParseError::InvalidFormat {
                    location: format!("command.{}.run", cmd_name),
                    message: format!(
                        "references unknown agent '{}'; available agents: {}",
                        agent_name,
                        sorted_keys(&runbook.agents),
                    ),
                });
            }
        }
        if let Some(pl_name) = cmd.run.job_name() {
            if !runbook.jobs.contains_key(pl_name) {
                return Err(ParseError::InvalidFormat {
                    location: format!("command.{}.run", cmd_name),
                    message: format!(
                        "references unknown job '{}'; available jobs: {}",
                        pl_name,
                        sorted_keys(&runbook.jobs),
                    ),
                });
            }
        }
    }

    Ok(())
}
