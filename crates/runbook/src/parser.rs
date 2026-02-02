// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runbook parsing (TOML, HCL, and JSON)

use crate::{
    ActionTrigger, AgentDef, ArgSpecError, CommandDef, PipelineDef, PrimeDef, QueueDef, QueueType,
    RunDirective, WorkerDef,
};
use oj_shell as shell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
pub struct Runbook {
    #[serde(default, alias = "command")]
    pub commands: HashMap<String, CommandDef>,
    #[serde(default, alias = "pipeline")]
    pub pipelines: HashMap<String, PipelineDef>,
    #[serde(default, alias = "agent")]
    pub agents: HashMap<String, AgentDef>,
    #[serde(default, alias = "queue")]
    pub queues: HashMap<String, QueueDef>,
    #[serde(default, alias = "worker")]
    pub workers: HashMap<String, WorkerDef>,
}

impl Runbook {
    /// Get a command definition by name
    pub fn get_command(&self, name: &str) -> Option<&CommandDef> {
        self.commands.get(name)
    }

    /// Get a pipeline definition by name
    pub fn get_pipeline(&self, name: &str) -> Option<&PipelineDef> {
        self.pipelines.get(name)
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
}

/// Format a shell parse error as a diagnostic with source snippet.
fn shell_diagnostic(err: &shell::ParseError, source: &str) -> String {
    err.diagnostic(source).unwrap_or_else(|| err.to_string())
}

/// Format a shell validation error as a diagnostic with source snippet.
fn validation_diagnostic(err: &shell::ValidationError, source: &str) -> String {
    err.diagnostic(source)
}

/// Agent commands recognized at parse time.
///
/// Commands not in this list will produce a parse error, preventing typos and
/// ensuring only supported agent adapters are referenced.
const SUPPORTED_AGENT_COMMANDS: &[&str] = &["claude", "claudeless"];

/// Claude/claudeless CLI options that take a value argument.
///
/// These options consume the next argument as their value, so we need to know
/// them to correctly identify positional arguments in the command.
/// Options with `=` syntax (e.g., `--model=haiku`) are handled automatically.
const CLAUDE_OPTIONS_WITH_VALUES: &[&str] = &[
    // claude options
    "add-dir",
    "agent",
    "agents",
    "allowed-tools",
    "allowedTools",
    "append-system-prompt",
    "betas",
    "debug",
    "debug-file",
    "disallowed-tools",
    "disallowedTools",
    "fallback-model",
    "file",
    "from-pr",
    "input-format",
    "json-schema",
    "max-budget-usd",
    "mcp-config",
    "model",
    "output-format",
    "permission-mode",
    "plugin-dir",
    "resume",
    "session-id",
    "setting-sources",
    "settings",
    "system-prompt",
    "tools",
    // claudeless options
    "scenario",
    "tool-mode",
];

/// Validate a shell command string, returning an error with context on failure.
///
/// Template variables like `${name}` are replaced with placeholder strings before
/// validation to avoid conflicts with shell brace group syntax.
fn validate_shell_command(command: &str, location: &str) -> Result<(), ParseError> {
    // Replace template variables with placeholders to avoid brace conflicts
    let normalized = crate::template::VAR_PATTERN.replace_all(command, "_VAR_");
    let source_text = normalized.to_string();
    let ast = shell::Parser::parse(&normalized).map_err(|inner| ParseError::ShellError {
        location: location.to_string(),
        inner: Box::new(inner),
        source_text: source_text.clone(),
    })?;
    if let Err(errors) = shell::validate(&ast) {
        if let Some(inner) = errors.into_iter().next() {
            return Err(ParseError::ShellValidation {
                location: location.to_string(),
                inner: Box::new(inner),
                source_text,
            });
        }
    }
    Ok(())
}

/// Validate a duration string like "30s", "5m", "1h", "0s".
fn validate_duration_str(s: &str) -> Result<(), String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".to_string());
    }

    let (num_str, suffix) = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| (&s[..i], &s[i..]))
        .unwrap_or((s, ""));

    let _num: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number in duration: {}", s))?;

    match suffix.trim() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" | "m" | "min" | "mins" | "minute"
        | "minutes" | "h" | "hr" | "hrs" | "hour" | "hours" | "d" | "day" | "days" => Ok(()),
        other => Err(format!("unknown duration suffix: {}", other)),
    }
}

/// Validate that an agent's run command uses a recognized agent command.
///
/// Parses the shell AST and extracts the first command name (taking basename
/// to handle absolute paths like `/usr/local/bin/claude`). If the command name
/// can't be statically determined (e.g. variable-only name), validation is skipped.
///
/// If `has_prompt` is true, positional arguments are rejected since the system
/// will append the prompt automatically.
fn validate_agent_command(
    command: &str,
    location: &str,
    has_prompt: bool,
) -> Result<(), ParseError> {
    let normalized = crate::template::VAR_PATTERN.replace_all(command, "_VAR_");
    let source_text = normalized.to_string();
    let ast = shell::Parser::parse(&normalized).map_err(|inner| ParseError::ShellError {
        location: location.to_string(),
        inner: Box::new(inner),
        source_text,
    })?;

    // Walk to the first SimpleCommand in the AST
    let first_cmd = ast.commands.first().map(|and_or| &and_or.first.command);

    let simple = match first_cmd {
        Some(shell::Command::Simple(cmd)) => cmd,
        Some(shell::Command::Pipeline(p)) => match p.commands.first() {
            Some(cmd) => cmd,
            None => return Ok(()),
        },
        _ => return Ok(()),
    };

    // Extract the command name if the first part is a literal
    let name = match simple.name.parts.first() {
        Some(shell::WordPart::Literal { value, .. }) => value.as_str(),
        _ => return Ok(()), // Variable or substitution — skip validation
    };

    // Take basename to handle absolute paths
    let basename = name.rsplit('/').next().unwrap_or(name);

    if !SUPPORTED_AGENT_COMMANDS.contains(&basename) {
        return Err(ParseError::InvalidFormat {
            location: location.to_string(),
            message: format!(
                "unrecognized agent command '{}'; must be one of: {}",
                basename,
                SUPPORTED_AGENT_COMMANDS.join(", "),
            ),
        });
    }

    // Reject --session-id flag (the system adds it automatically)
    if simple.has_long_option("session-id") {
        return Err(ParseError::InvalidFormat {
            location: location.to_string(),
            message: format!(
                "{} command must not include '--session-id' (added automatically)",
                basename,
            ),
        });
    }

    // Reject positional arguments when prompt field is configured (system appends it)
    if has_prompt
        && !simple
            .positional_args(CLAUDE_OPTIONS_WITH_VALUES)
            .is_empty()
    {
        return Err(ParseError::InvalidFormat {
            location: location.to_string(),
            message: format!(
                "{} command must not include positional arguments when prompt is configured (use prompt field or inline, not both)",
                basename,
            ),
        });
    }

    Ok(())
}

/// Parse a runbook from TOML content (convenience wrapper)
pub fn parse_runbook(content: &str) -> Result<Runbook, ParseError> {
    parse_runbook_with_format(content, Format::Toml)
}

/// Parse a runbook from the given content in the specified format
pub fn parse_runbook_with_format(content: &str, format: Format) -> Result<Runbook, ParseError> {
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
    for (name, pipeline) in &mut runbook.pipelines {
        pipeline.name = name.clone();
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

    // 3. Validation — step names must not be empty
    for (pipeline_name, pipeline) in &runbook.pipelines {
        for (i, step) in pipeline.steps.iter().enumerate() {
            if step.name.is_empty() {
                return Err(ParseError::InvalidFormat {
                    location: format!("pipeline.{}.step[{}]", pipeline_name, i),
                    message: "step name is required".to_string(),
                });
            }
        }
    }

    // 4. Validation — shell command syntax and agent command checks
    for (name, cmd) in &runbook.commands {
        if let RunDirective::Shell(ref shell_cmd) = cmd.run {
            validate_shell_command(shell_cmd, &format!("command.{}.run", name))?;
        }
    }

    for (pipeline_name, pipeline) in &runbook.pipelines {
        for (i, step) in pipeline.steps.iter().enumerate() {
            if let RunDirective::Shell(ref shell_cmd) = step.run {
                validate_shell_command(
                    shell_cmd,
                    &format!("pipeline.{}.step[{}]({}).run", pipeline_name, i, step.name),
                )?;
            }
        }
    }

    for (name, agent) in &runbook.agents {
        let has_prompt = agent.prompt.is_some() || agent.prompt_file.is_some();

        if !agent.run.is_empty() {
            let run_location = format!("agent.{}.run", name);
            validate_shell_command(&agent.run, &run_location)?;
            validate_agent_command(&agent.run, &run_location, has_prompt)?;
        }

        if let Some(PrimeDef::Commands(cmds)) = &agent.prime {
            for (i, cmd) in cmds.iter().enumerate() {
                validate_shell_command(cmd, &format!("agent.{}.prime[{}]", name, i))?;
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

    // 6. Validate worker cross-references
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
        if !runbook.pipelines.contains_key(&worker.handler.pipeline) {
            return Err(ParseError::InvalidFormat {
                location: format!("worker.{}.handler.pipeline", name),
                message: format!(
                    "references unknown pipeline '{}'; available pipelines: {}",
                    worker.handler.pipeline,
                    runbook
                        .pipelines
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            });
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

    Ok(runbook)
}

#[cfg(test)]
#[path = "parser_tests/mod.rs"]
mod tests;
