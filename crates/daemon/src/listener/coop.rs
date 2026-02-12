// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Coop agent process utilities for IPC handlers.

use std::io::Write;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::adapters::agent::LocalAdapter;
use crate::storage::MaterializedState;

/// Kill agent processes tracked by this daemon instance, concurrently.
///
/// Collects agent IDs from non-terminal jobs and crew. Each kill is
/// spawned as a tokio task for O(1) latency regardless of agent count.
///
/// Uses unbuffered stderr writes instead of tracing because the non-blocking
/// tracing appender may not flush before the CLI's exit timer force-kills
/// the daemon process.
pub(super) async fn kill_state_agents(
    state: &Arc<Mutex<MaterializedState>>,
    state_dir: &std::path::Path,
) {
    let agent_ids: Vec<String> = {
        let state = state.lock();
        let mut ids = Vec::new();

        // Collect agent IDs from non-terminal jobs
        for job in state.jobs.values() {
            if !job.is_terminal() {
                if let Some(record) = job.step_history.iter().rfind(|r| r.name == job.step) {
                    if let Some(ref aid) = record.agent_id {
                        ids.push(aid.clone());
                    }
                }
            }
        }

        // Collect agent IDs from non-terminal crew
        for run in state.crew.values() {
            if !run.status.is_terminal() {
                if let Some(ref aid) = run.agent_id {
                    ids.push(aid.clone());
                }
            }
        }

        ids
    };

    if agent_ids.is_empty() {
        return;
    }

    let count = agent_ids.len();
    let mut handles = Vec::with_capacity(count);
    for id in &agent_ids {
        let id = id.clone();
        let dir = state_dir.to_path_buf();
        handles.push(tokio::spawn(async move {
            LocalAdapter::kill_agent(&dir, &id).await;
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }

    // Unbuffered write — survives force-kill better than tracing
    let _ = writeln!(std::io::stderr(), "ojd: killed {} agent processes on shutdown", count);
}
