// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron request handlers.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::{Event, IdGen, PipelineId, UuidIdGen};
use oj_storage::MaterializedState;

use crate::event_bus::EventBus;
use crate::protocol::Response;

use super::suggest;
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
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook to validate cron exists.
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_cron(root, cron_name),
        || {
            suggest_for_cron(
                Some(project_root),
                cron_name,
                namespace,
                "oj cron start",
                state,
            )
        },
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_root = &effective_root;

    // Validate cron definition exists
    let cron_def = match runbook.get_cron(cron_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown cron: {}", cron_name),
            })
        }
    };

    // Validate run is a pipeline or agent reference
    let (pipeline_name, run_target) = match &cron_def.run {
        oj_runbook::RunDirective::Pipeline { pipeline } => {
            if runbook.get_pipeline(pipeline).is_none() {
                return Ok(Response::Error {
                    message: format!(
                        "cron '{}' references unknown pipeline '{}'",
                        cron_name, pipeline
                    ),
                });
            }
            (pipeline.clone(), format!("pipeline:{}", pipeline))
        }
        oj_runbook::RunDirective::Agent { agent, .. } => {
            if runbook.get_agent(agent).is_none() {
                return Ok(Response::Error {
                    message: format!("cron '{}' references unknown agent '{}'", cron_name, agent),
                });
            }
            (String::new(), format!("agent:{}", agent))
        }
        _ => {
            return Ok(Response::Error {
                message: format!(
                    "cron '{}' run must reference a pipeline or agent",
                    cron_name
                ),
            })
        }
    };

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
        run_target,
        namespace: namespace.to_string(),
    };

    event_bus
        .send(event.clone())
        .map_err(|_| ConnectionError::WalError)?;

    // Apply to materialized state before responding so queries see it
    // immediately. apply_event is idempotent so the second apply when the
    // main loop processes this event from the WAL is harmless.
    {
        let mut state = state.lock();
        state.apply_event(&event);
    }

    Ok(Response::CronStarted {
        cron_name: cron_name.to_string(),
    })
}

/// Handle a CronStop request.
pub(super) fn handle_cron_stop(
    cron_name: &str,
    namespace: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
    project_root: Option<&Path>,
) -> Result<Response, ConnectionError> {
    // Check if cron exists in state
    let scoped = if namespace.is_empty() {
        cron_name.to_string()
    } else {
        format!("{}/{}", namespace, cron_name)
    };
    let exists = {
        let state = state.lock();
        state.crons.contains_key(&scoped)
    };
    if !exists {
        let hint = suggest_for_cron(project_root, cron_name, namespace, "oj cron stop", state);
        return Ok(Response::Error {
            message: format!("unknown cron: {}{}", cron_name, hint),
        });
    }

    let event = Event::CronStopped {
        cron_name: cron_name.to_string(),
        namespace: namespace.to_string(),
    };

    event_bus
        .send(event.clone())
        .map_err(|_| ConnectionError::WalError)?;

    // Apply to materialized state before responding so queries see it
    // immediately. apply_event is idempotent.
    {
        let mut state = state.lock();
        state.apply_event(&event);
    }

    Ok(Response::Ok)
}

/// Handle a CronOnce request — run the cron's pipeline once immediately.
pub(super) async fn handle_cron_once(
    project_root: &Path,
    namespace: &str,
    cron_name: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Load runbook to validate cron exists.
    let (runbook, effective_root) = match super::load_runbook_with_fallback(
        project_root,
        namespace,
        state,
        |root| load_runbook_for_cron(root, cron_name),
        || {
            suggest_for_cron(
                Some(project_root),
                cron_name,
                namespace,
                "oj cron once",
                state,
            )
        },
    ) {
        Ok(result) => result,
        Err(resp) => return Ok(resp),
    };
    let project_root = &effective_root;

    // Validate cron definition exists
    let cron_def = match runbook.get_cron(cron_name) {
        Some(def) => def,
        None => {
            return Ok(Response::Error {
                message: format!("unknown cron: {}", cron_name),
            })
        }
    };

    // Validate run is a pipeline or agent reference and build event
    let (pipeline_name, run_target) = match &cron_def.run {
        oj_runbook::RunDirective::Pipeline { pipeline } => {
            if runbook.get_pipeline(pipeline).is_none() {
                return Ok(Response::Error {
                    message: format!(
                        "cron '{}' references unknown pipeline '{}'",
                        cron_name, pipeline
                    ),
                });
            }
            (pipeline.clone(), format!("pipeline:{}", pipeline))
        }
        oj_runbook::RunDirective::Agent { agent, .. } => {
            if runbook.get_agent(agent).is_none() {
                return Ok(Response::Error {
                    message: format!("cron '{}' references unknown agent '{}'", cron_name, agent),
                });
            }
            (String::new(), format!("agent:{}", agent))
        }
        _ => {
            return Ok(Response::Error {
                message: format!(
                    "cron '{}' run must reference a pipeline or agent",
                    cron_name
                ),
            })
        }
    };

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

    let is_agent = run_target.starts_with("agent:");

    if is_agent {
        let agent_name = run_target.strip_prefix("agent:").unwrap_or("").to_string();
        let agent_run_id = UuidIdGen.next();

        let event = Event::CronOnce {
            cron_name: cron_name.to_string(),
            pipeline_id: PipelineId::new(""),
            pipeline_name: String::new(),
            pipeline_kind: String::new(),
            agent_run_id: Some(agent_run_id.clone()),
            agent_name: Some(agent_name.clone()),
            project_root: project_root.to_path_buf(),
            runbook_hash: runbook_hash.clone(),
            run_target,
            namespace: namespace.to_string(),
        };

        event_bus
            .send(event)
            .map_err(|_| ConnectionError::WalError)?;

        Ok(Response::CommandStarted {
            pipeline_id: agent_run_id,
            pipeline_name: format!("agent:{}", agent_name),
        })
    } else {
        // Generate pipeline ID
        let pipeline_id = PipelineId::new(UuidIdGen.next());
        let pipeline_display_name = oj_runbook::pipeline_display_name(
            &pipeline_name,
            &pipeline_id.as_str()[..8.min(pipeline_id.as_str().len())],
            namespace,
        );

        // Emit CronOnce event to create pipeline via the cron code path
        let event = Event::CronOnce {
            cron_name: cron_name.to_string(),
            pipeline_id: pipeline_id.clone(),
            pipeline_name: pipeline_display_name.clone(),
            pipeline_kind: pipeline_name.clone(),
            agent_run_id: None,
            agent_name: None,
            project_root: project_root.to_path_buf(),
            runbook_hash: runbook_hash.clone(),
            run_target,
            namespace: namespace.to_string(),
        };

        event_bus
            .send(event)
            .map_err(|_| ConnectionError::WalError)?;

        Ok(Response::CommandStarted {
            pipeline_id: pipeline_id.to_string(),
            pipeline_name: pipeline_display_name,
        })
    }
}

/// Handle a CronRestart request: stop (if running), reload runbook, start.
pub(super) fn handle_cron_restart(
    project_root: &Path,
    namespace: &str,
    cron_name: &str,
    event_bus: &EventBus,
    state: &Arc<Mutex<MaterializedState>>,
) -> Result<Response, ConnectionError> {
    // Stop cron if it exists in state
    let scoped = if namespace.is_empty() {
        cron_name.to_string()
    } else {
        format!("{}/{}", namespace, cron_name)
    };
    let exists = {
        let state = state.lock();
        state.crons.contains_key(&scoped)
    };
    if exists {
        let stop_event = Event::CronStopped {
            cron_name: cron_name.to_string(),
            namespace: namespace.to_string(),
        };
        event_bus
            .send(stop_event)
            .map_err(|_| ConnectionError::WalError)?;
    }

    // Start with fresh runbook
    handle_cron_start(project_root, namespace, cron_name, event_bus, state)
}

#[cfg(test)]
#[path = "crons_tests.rs"]
mod tests;

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

/// Generate a "did you mean" suggestion for a cron name.
fn suggest_for_cron(
    project_root: Option<&Path>,
    cron_name: &str,
    namespace: &str,
    command_prefix: &str,
    state: &Arc<Mutex<MaterializedState>>,
) -> String {
    // 1. Collect all cron names from runbooks (if project_root available)
    if let Some(root) = project_root {
        let runbook_dir = root.join(".oj/runbooks");
        let all_crons = oj_runbook::collect_all_crons(&runbook_dir).unwrap_or_default();
        let candidates: Vec<&str> = all_crons.iter().map(|(name, _)| name.as_str()).collect();

        let similar = suggest::find_similar(cron_name, &candidates);
        if !similar.is_empty() {
            return suggest::format_suggestion(&similar);
        }
    }

    // 2. Try suggestions from daemon state (active/stopped crons in current namespace)
    {
        let state = state.lock();
        let state_candidates: Vec<&str> = state
            .crons
            .values()
            .filter(|c| c.namespace == namespace)
            .map(|c| c.name.as_str())
            .collect();
        let similar = suggest::find_similar(cron_name, &state_candidates);
        if !similar.is_empty() {
            return suggest::format_suggestion(&similar);
        }
    }

    // 3. Check for wrong project (cross-namespace)
    let state = state.lock();
    if let Some(other_ns) =
        suggest::find_in_other_namespaces(suggest::ResourceType::Cron, cron_name, namespace, &state)
    {
        return suggest::format_cross_project_suggestion(command_prefix, cron_name, &other_ns);
    }

    String::new()
}
