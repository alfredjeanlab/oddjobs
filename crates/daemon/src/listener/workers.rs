// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker request handlers.

use std::path::Path;

use oj_core::Event;

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::ConnectionError;

/// Handle a WorkerStart request.
///
/// Idempotent: always emits `WorkerStarted`. The runtime's `handle_worker_started`
/// overwrites any existing in-memory state, so repeated starts are safe and also
/// serve as a wake (triggering an initial poll).
pub(super) fn handle_worker_start(
    project_root: &Path,
    worker_name: &str,
    event_bus: &EventBus,
) -> Result<Response, ConnectionError> {
    // Load runbook to validate worker exists
    let runbook = match load_runbook_for_worker(project_root, worker_name) {
        Ok(rb) => rb,
        Err(e) => return Ok(Response::Error { message: e }),
    };

    // Validate worker definition exists
    let worker_def = match runbook.get_worker(worker_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown worker: {}", worker_name),
            })
        }
    };

    // Validate referenced queue exists
    if runbook.get_queue(&worker_def.source.queue).is_none() {
        return Ok(Response::Error {
            message: format!(
                "worker '{}' references unknown queue '{}'",
                worker_name, worker_def.source.queue
            ),
        });
    }

    // Validate referenced pipeline exists
    if runbook.get_pipeline(&worker_def.handler.pipeline).is_none() {
        return Ok(Response::Error {
            message: format!(
                "worker '{}' references unknown pipeline '{}'",
                worker_name, worker_def.handler.pipeline
            ),
        });
    }

    // Serialize and hash the runbook for WAL storage
    let runbook_json = match serde_json::to_value(&runbook) {
        Ok(v) => v,
        Err(e) => {
            return Ok(Response::Error {
                message: format!("failed to serialize runbook: {}", e),
            })
        }
    };
    let runbook_hash = {
        use sha2::{Digest, Sha256};
        let canonical = match serde_json::to_string(&runbook_json) {
            Ok(s) => s,
            Err(e) => {
                return Ok(Response::Error {
                    message: format!("failed to serialize runbook: {}", e),
                })
            }
        };
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
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

    // Emit WorkerStarted event
    let event = Event::WorkerStarted {
        worker_name: worker_name.to_string(),
        project_root: project_root.to_path_buf(),
        runbook_hash,
        queue_name: worker_def.source.queue.clone(),
        concurrency: worker_def.concurrency,
    };

    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::WorkerStarted {
        worker_name: worker_name.to_string(),
    })
}

/// Handle a WorkerStop request.
pub(super) fn handle_worker_stop(
    worker_name: &str,
    event_bus: &EventBus,
) -> Result<Response, ConnectionError> {
    let event = Event::WorkerStopped {
        worker_name: worker_name.to_string(),
    };

    event_bus
        .send(event)
        .map_err(|_| ConnectionError::WalError)?;

    Ok(Response::Ok)
}

#[cfg(test)]
#[path = "workers_tests.rs"]
mod tests;

/// Load a runbook that contains the given worker name.
fn load_runbook_for_worker(
    project_root: &Path,
    worker_name: &str,
) -> Result<oj_runbook::Runbook, String> {
    let runbook_dir = project_root.join(".oj/runbooks");
    oj_runbook::find_runbook_by_worker(&runbook_dir, worker_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no runbook found containing worker '{}'", worker_name))
}
