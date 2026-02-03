// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! ListProjects query handler.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::StepOutcome;
use oj_storage::MaterializedState;

use crate::protocol::{ProjectSummary, Response};

pub(super) fn handle_list_projects(state: &Arc<Mutex<MaterializedState>>) -> Response {
    let state = state.lock();

    let mut ns_roots: BTreeMap<String, std::path::PathBuf> = BTreeMap::new();
    let mut ns_workers: BTreeMap<String, usize> = BTreeMap::new();
    let mut ns_crons: BTreeMap<String, usize> = BTreeMap::new();
    let mut ns_pipelines: BTreeMap<String, usize> = BTreeMap::new();
    let mut ns_agents: BTreeMap<String, usize> = BTreeMap::new();

    for w in state.workers.values() {
        if w.status == "running" {
            ns_roots
                .entry(w.namespace.clone())
                .or_insert_with(|| w.project_root.clone());
            *ns_workers.entry(w.namespace.clone()).or_default() += 1;
        }
    }

    for c in state.crons.values() {
        if c.status == "running" {
            ns_roots
                .entry(c.namespace.clone())
                .or_insert_with(|| c.project_root.clone());
            *ns_crons.entry(c.namespace.clone()).or_default() += 1;
        }
    }

    for p in state.pipelines.values() {
        if !p.is_terminal() {
            *ns_pipelines.entry(p.namespace.clone()).or_default() += 1;

            // Count active agents from the current step
            if let Some(last_step) = p.step_history.last() {
                if last_step.agent_id.is_some()
                    && matches!(
                        &last_step.outcome,
                        StepOutcome::Running | StepOutcome::Waiting(_)
                    )
                {
                    *ns_agents.entry(p.namespace.clone()).or_default() += 1;
                }
            }

            // Use stopped workers/crons as fallback for project_root
            if !ns_roots.contains_key(&p.namespace) {
                if let Some(w) = state.workers.values().find(|w| w.namespace == p.namespace) {
                    ns_roots.insert(w.namespace.clone(), w.project_root.clone());
                }
            }
            if !ns_roots.contains_key(&p.namespace) {
                if let Some(c) = state.crons.values().find(|c| c.namespace == p.namespace) {
                    ns_roots.insert(c.namespace.clone(), c.project_root.clone());
                }
            }
        }
    }

    // Build project summaries for namespaces with active entities
    let mut all_ns: HashSet<String> = HashSet::new();
    all_ns.extend(ns_workers.keys().cloned());
    all_ns.extend(ns_crons.keys().cloned());
    all_ns.extend(ns_pipelines.keys().cloned());

    let mut projects: Vec<ProjectSummary> = all_ns
        .into_iter()
        .map(|ns| ProjectSummary {
            root: ns_roots.get(&ns).cloned().unwrap_or_default(),
            active_pipelines: ns_pipelines.get(&ns).copied().unwrap_or(0),
            active_agents: ns_agents.get(&ns).copied().unwrap_or(0),
            workers: ns_workers.get(&ns).copied().unwrap_or(0),
            crons: ns_crons.get(&ns).copied().unwrap_or(0),
            name: ns,
        })
        .collect();
    projects.sort_by(|a, b| a.name.cmp(&b.name));

    Response::Projects { projects }
}
