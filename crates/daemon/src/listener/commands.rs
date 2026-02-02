// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Command handlers.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::{IdGen, PipelineId, UuidIdGen};
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::ConnectionError;

/// Handle a RunCommand request.
// TODO(refactor): group run command parameters into a struct
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_run_command(
    project_root: &Path,
    invoke_dir: &Path,
    namespace: &str,
    command: &str,
    args: &[String],
    named_args: &HashMap<String, String>,
    event_bus: &EventBus,
    _state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook from project
    let runbook: oj_runbook::Runbook = match load_runbook(project_root, command) {
        Ok(rb) => rb,
        Err(e) => {
            return Ok(Response::Error {
                message: format!("failed to load runbook: {}", e),
            })
        }
    };

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

    // Get pipeline name from command definition (shell commands use the command name)
    let pipeline_name = cmd_def.run.pipeline_name().unwrap_or(command).to_string();

    // Generate pipeline ID
    let pipeline_id = PipelineId::new(UuidIdGen.next());

    // Parse arguments
    let parsed_args = cmd_def.parse_args(args, &named);

    // Send event to engine
    let event = oj_core::Event::CommandRun {
        pipeline_id: pipeline_id.clone(),
        pipeline_name: pipeline_name.clone(),
        project_root: project_root.to_path_buf(),
        invoke_dir: invoke_dir.to_path_buf(),
        namespace: namespace.to_string(),
        command: command.to_string(),
        args: parsed_args,
    };

    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::CommandStarted {
        pipeline_id: pipeline_id.to_string(),
        pipeline_name,
    })
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
