// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron request handlers.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::{Event, IdGen, PipelineId, UuidIdGen};
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::workers::hash_runbook;
use super::ConnectionError;

/// Handle a CronStart request.
///
/// Idempotent: always emits `CronStarted`. The runtime's `handle_cron_started`
/// overwrites any existing in-memory state, so repeated starts are safe and also
/// serve to update the interval if the runbook changed.
pub(super) fn handle_cron_start(
    project_root: &Path,
    namespace: &str,
    cron_name: &str,
    event_bus: &EventBus,
) -> Result<Response, ConnectionError> {
    // Load runbook to validate cron exists
    let runbook = match load_runbook_for_cron(project_root, cron_name) {
        Ok(rb) => rb,
        Err(e) => return Ok(Response::Error { message: e }),
    };

    // Validate cron definition exists
    let cron_def = match runbook.get_cron(cron_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown cron: {}", cron_name),
            })
        }
    };

    // Validate run is a pipeline reference
    let pipeline_name = match cron_def.run.pipeline_name() {
        Some(p) => p.to_string(),
        None => {
            return Ok(Response::Error {
                message: format!("cron '{}' run must reference a pipeline", cron_name),
            })
        }
    };

    // Validate referenced pipeline exists
    if runbook.get_pipeline(&pipeline_name).is_none() {
        return Ok(Response::Error {
            message: format!(
                "cron '{}' references unknown pipeline '{}'",
                cron_name, pipeline_name
            ),
        });
    }

    // Serialize and hash the runbook for WAL storage
    let (runbook_json, runbook_hash) = match hash_runbook(&runbook) {
        Ok(result) => result,
        Err(msg) => return Ok(Response::Error { message: msg }),
    };

    // Emit RunbookLoaded for WAL persistence
    let runbook_event = Event::RunbookLoaded {
        hash: runbook_hash.clone(),
        version: 1,
        runbook: runbook_json,
    };
    event_bus
        .send(runbook_event)
        .map_err(|_| ConnectionError::WalError)?;

    // Emit CronStarted event
    let event = Event::CronStarted {
        cron_name: cron_name.to_string(),
        project_root: project_root.to_path_buf(),
        runbook_hash,
        interval: cron_def.interval.clone(),
        pipeline_name,
        namespace: namespace.to_string(),
    };

    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::CronStarted {
        cron_name: cron_name.to_string(),
    })
}

/// Handle a CronStop request.
pub(super) fn handle_cron_stop(
    cron_name: &str,
    namespace: &str,
    event_bus: &EventBus,
) -> Result<Response, ConnectionError> {
    let event = Event::CronStopped {
        cron_name: cron_name.to_string(),
        namespace: namespace.to_string(),
    };

    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::Ok)
}

/// Handle a CronOnce request — run the cron's pipeline once immediately.
pub(super) async fn handle_cron_once(
    project_root: &Path,
    namespace: &str,
    cron_name: &str,
    event_bus: &EventBus,
    _state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook to validate cron exists
    let runbook = match load_runbook_for_cron(project_root, cron_name) {
        Ok(rb) => rb,
        Err(e) => return Ok(Response::Error { message: e }),
    };

    // Validate cron definition exists
    let cron_def = match runbook.get_cron(cron_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown cron: {}", cron_name),
            })
        }
    };

    // Validate run is a pipeline reference
    let pipeline_name = match cron_def.run.pipeline_name() {
        Some(p) => p.to_string(),
        None => {
            return Ok(Response::Error {
                message: format!("cron '{}' run must reference a pipeline", cron_name),
            })
        }
    };

    // Validate referenced pipeline exists
    if runbook.get_pipeline(&pipeline_name).is_none() {
        return Ok(Response::Error {
            message: format!(
                "cron '{}' references unknown pipeline '{}'",
                cron_name, pipeline_name
            ),
        });
    }

    // Serialize and hash the runbook for WAL storage
    let (runbook_json, runbook_hash) = match hash_runbook(&runbook) {
        Ok(result) => result,
        Err(msg) => return Ok(Response::Error { message: msg }),
    };

    // Emit RunbookLoaded for WAL persistence
    let runbook_event = Event::RunbookLoaded {
        hash: runbook_hash.clone(),
        version: 1,
        runbook: runbook_json,
    };
    event_bus
        .send(runbook_event)
        .map_err(|_| ConnectionError::WalError)?;

    // Generate pipeline ID
    let pipeline_id = PipelineId::new(UuidIdGen.next());
    let pipeline_display_name = oj_runbook::pipeline_display_name(
        &pipeline_name,
        &pipeline_id.as_str()[..8.min(pipeline_id.as_str().len())],
        namespace,
    );

    // Emit CommandRun-style event to create pipeline directly
    let event = Event::CommandRun {
        pipeline_id: pipeline_id.clone(),
        pipeline_name: pipeline_display_name.clone(),
        project_root: project_root.to_path_buf(),
        invoke_dir: project_root.to_path_buf(),
        namespace: namespace.to_string(),
        command: pipeline_name.clone(),
        args: HashMap::new(),
    };

    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::CommandStarted {
        pipeline_id: pipeline_id.to_string(),
        pipeline_name: pipeline_display_name,
    })
}

/// Load a runbook that contains the given cron name.
fn load_runbook_for_cron(
    project_root: &Path,
    cron_name: &str,
) -> Result<oj_runbook::Runbook, String> {
    let runbook_dir = project_root.join(".oj/runbooks");
    oj_runbook::find_runbook_by_cron(&runbook_dir, cron_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no runbook found containing cron '{}'", cron_name))
}
