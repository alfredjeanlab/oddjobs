// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Command handlers.

use std::collections::HashMap;
use std::path::Path;

use oj_core::{IdGen, JobId, UuidIdGen};

use crate::protocol::Response;

use super::mutations::emit;
use super::suggest;
use super::ConnectionError;
use super::ListenCtx;

/// Parameters for handling a run command request.
pub(super) struct RunCommandParams<'a> {
    pub project_root: &'a Path,
    pub invoke_dir: &'a Path,
    pub namespace: &'a str,
    pub command: &'a str,
    pub args: &'a [String],
    pub named_args: &'a HashMap<String, String>,
    pub ctx: &'a ListenCtx,
}

/// Handle a RunCommand request.
pub(super) async fn handle_run_command(
    params: RunCommandParams<'_>,
) -> Result<Response, ConnectionError> {
    let RunCommandParams {
        project_root,
        invoke_dir,
        namespace,
        command,
        args,
        named_args,
        ctx,
    } = params;
    // Load runbook from project (with --project fallback and suggest hints)
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        &ctx.state,
        |root| load_runbook(root, command),
        || {
            let runbook_dir = project_root.join(".oj/runbooks");
            suggest::suggest_for_resource(
                command,
                namespace,
                "oj run",
                &ctx.state,
                suggest::ResourceType::Command,
                || {
                    oj_runbook::collect_all_commands(&runbook_dir)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(name, _)| name)
                        .collect()
                },
                |_| Vec::new(),
            )
        },
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_root = &effective_root;

    // Get command definition
    let cmd_def: &oj_runbook::CommandDef = match runbook.get_command(command) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown command: {}", command),
            })
        }
    };

    // Validate arguments
    let named: HashMap<String, String> = named_args.clone();
    if let Err(e) = cmd_def.validate_args(args, &named) {
        return Ok(Response::Error {
            message: e.to_string(),
        });
    }

    // Detect if this is a standalone agent command
    let is_agent = matches!(&cmd_def.run, oj_runbook::RunDirective::Agent { .. });
    let agent_name_if_standalone =
        if let oj_runbook::RunDirective::Agent { agent, .. } = &cmd_def.run {
            Some(agent.clone())
        } else {
            None
        };

    // Get job name from command definition (shell commands use the command name)
    let job_name = cmd_def.run.job_name().unwrap_or(command).to_string();

    // Generate job ID
    let job_id = JobId::new(UuidIdGen.next());

    // Parse arguments
    let parsed_args = cmd_def.parse_args(args, &named);

    // Send event to engine
    let event = oj_core::Event::CommandRun {
        job_id: job_id.clone(),
        job_name: job_name.clone(),
        project_root: project_root.to_path_buf(),
        invoke_dir: invoke_dir.to_path_buf(),
        namespace: namespace.to_string(),
        command: command.to_string(),
        args: parsed_args,
    };

    emit(&ctx.event_bus, event)?;

    if is_agent {
        // For standalone agent commands, return AgentRunStarted
        // The engine generates the actual agent_run_id, but the daemon needs to
        // return a response immediately. We use the job_id as a correlation
        // key — the engine's command handler will create the agent_run.
        Ok(Response::AgentRunStarted {
            agent_run_id: job_id.to_string(),
            agent_name: agent_name_if_standalone.unwrap_or_default(),
        })
    } else {
        Ok(Response::CommandStarted {
            job_id: job_id.to_string(),
            job_name,
        })
    }
}

/// Load a runbook from a project root by scanning all .toml files.
fn load_runbook(project_root: &Path, name: &str) -> Result<oj_runbook::Runbook, String> {
    let runbook_dir = project_root.join(".oj/runbooks");
    oj_runbook::find_runbook_by_command(&runbook_dir, name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("unknown command: {}", name))
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
