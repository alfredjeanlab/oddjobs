// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Command handlers.

use std::collections::HashMap;
use std::path::Path;

use oj_core::{CrewId, IdGen, JobId, OwnerId, UuidIdGen};

use crate::protocol::Response;

use super::mutations::emit;
use super::suggest;
use super::ConnectionError;
use super::ListenCtx;

/// Parameters for handling a run command request.
pub(super) struct RunCommandParams<'a> {
    pub project_path: &'a Path,
    pub invoke_dir: &'a Path,
    pub project: &'a str,
    pub command: &'a str,
    pub args: &'a [String],
    pub named_args: &'a HashMap<String, String>,
    pub ctx: &'a ListenCtx,
}

/// Handle a RunCommand request.
pub(super) async fn handle_run_command(
    params: RunCommandParams<'_>,
) -> Result<Response, ConnectionError> {
    let RunCommandParams { project_path, invoke_dir, project, command, args, named_args, ctx } =
        params;
    // Load runbook from project (with --project fallback and suggest hints)
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_path,
        project,
        &ctx.state,
        |root| load_runbook(root, command),
        || {
            let runbook_dir = project_path.join(".oj/runbooks");
            suggest::suggest_for_resource(
                command,
                project,
                "oj run",
                &ctx.state,
                suggest::ResourceType::Command,
                || {
                    oj_runbook::collect_all_commands(&runbook_dir)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(name, _, _)| name)
                        .collect()
                },
                |_| Vec::new(),
            )
        },
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_path = &effective_root;

    // Get command definition
    let cmd_def: &oj_runbook::CommandDef = match runbook.get_command(command) {
        Some(def) => def,
        None => return Ok(Response::Error { message: format!("unknown command: {}", command) }),
    };

    // Validate arguments
    let named: HashMap<String, String> = named_args.clone();
    if let Err(e) = cmd_def.validate_args(args, &named) {
        return Ok(Response::Error { message: e.to_string() });
    }

    // Generate the correct ID type upfront
    let (owner, name): (OwnerId, String) = match &cmd_def.run {
        oj_runbook::RunDirective::Job { .. } | oj_runbook::RunDirective::Shell(_) => {
            let id = JobId::new(UuidIdGen.next());
            let name = cmd_def.run.job_name().unwrap_or(command).to_string();
            (id.into(), name)
        }
        oj_runbook::RunDirective::Agent { agent, .. } => {
            let id = CrewId::new(UuidIdGen.next());
            (id.into(), agent.clone())
        }
    };

    // Parse arguments
    let parsed_args = cmd_def.parse_args(args, &named);

    // Send event to engine
    let event = oj_core::Event::CommandRun {
        owner: owner.clone(),
        name: name.clone(),
        project_path: project_path.to_path_buf(),
        invoke_dir: invoke_dir.to_path_buf(),
        project: project.to_string(),
        command: command.to_string(),
        args: parsed_args,
    };

    emit(&ctx.event_bus, event)?;

    match &owner {
        OwnerId::Crew(crew_id) => {
            Ok(Response::CrewStarted { crew_id: crew_id.to_string(), agent_name: name })
        }
        OwnerId::Job(job_id) => {
            Ok(Response::JobStarted { job_id: job_id.to_string(), job_name: name })
        }
    }
}

/// Load a runbook from a project root by scanning all .toml files.
fn load_runbook(project_path: &Path, name: &str) -> Result<oj_runbook::Runbook, String> {
    let runbook_dir = project_path.join(".oj/runbooks");
    oj_runbook::find_runbook_by_command(&runbook_dir, name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("unknown command: {}", name))
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
