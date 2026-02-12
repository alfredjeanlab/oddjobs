// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Import merge logic — prefix names, rename cross-references, merge entity maps.

use crate::parser::{ParseError, Runbook};
use std::collections::hash_map::Entry;
use std::collections::HashMap;

use super::types::ImportWarning;

/// Merge an imported runbook into the target runbook.
///
/// If `alias` is provided, all entity names in `source` are prefixed with `alias:`.
/// Internal cross-references within the imported runbook are also updated.
///
/// Conflict handling:
/// - Local entity vs import with same name → local wins (warning)
/// - Import A vs Import B with same name → error
pub fn merge_runbook(
    target: &mut Runbook,
    mut source: Runbook,
    alias: Option<&str>,
    import_source: &str,
) -> Result<Vec<ImportWarning>, ParseError> {
    let mut warnings = Vec::new();

    if let Some(prefix) = alias {
        prefix_names(&mut source, prefix);
    }

    // Merge each entity type
    merge_map(&mut target.commands, source.commands, "command", import_source, &mut warnings)?;
    merge_map(&mut target.jobs, source.jobs, "job", import_source, &mut warnings)?;
    merge_map(&mut target.agents, source.agents, "agent", import_source, &mut warnings)?;
    merge_map(&mut target.queues, source.queues, "queue", import_source, &mut warnings)?;
    merge_map(&mut target.workers, source.workers, "worker", import_source, &mut warnings)?;
    merge_map(&mut target.crons, source.crons, "cron", import_source, &mut warnings)?;

    Ok(warnings)
}

/// Merge a source map into a target map, with conflict handling.
fn merge_map<V>(
    target: &mut HashMap<String, V>,
    source: HashMap<String, V>,
    entity_type: &'static str,
    import_source: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), ParseError> {
    for (name, value) in source {
        match target.entry(name) {
            Entry::Occupied(e) => {
                // Local wins — emit warning
                warnings.push(ImportWarning::LocalOverride {
                    entity_type,
                    name: e.key().clone(),
                    source: import_source.to_string(),
                });
            }
            Entry::Vacant(e) => {
                e.insert(value);
            }
        }
    }
    Ok(())
}

/// Build a rename map: old_name → prefix:old_name.
fn build_entity_renames(
    keys: impl Iterator<Item = impl AsRef<str>>,
    prefix: &str,
) -> HashMap<String, String> {
    keys.map(|k| {
        let k = k.as_ref();
        (k.to_string(), format!("{}:{}", prefix, k))
    })
    .collect()
}

/// Update `.name` (or equivalent identity field) on all values in a map.
fn update_entity_names<V>(map: &mut HashMap<String, V>, mut set_name: impl FnMut(&mut V, &str)) {
    for (key, val) in map.iter_mut() {
        set_name(val, key);
    }
}

/// Prefix all entity names and update internal cross-references.
fn prefix_names(runbook: &mut Runbook, prefix: &str) {
    // Collect old→new name mappings for each entity type
    let cmd_renames = build_entity_renames(runbook.commands.keys(), prefix);
    let job_renames = build_entity_renames(runbook.jobs.keys(), prefix);
    let agent_renames = build_entity_renames(runbook.agents.keys(), prefix);
    let queue_renames = build_entity_renames(runbook.queues.keys(), prefix);
    let worker_renames = build_entity_renames(runbook.workers.keys(), prefix);
    let cron_renames = build_entity_renames(runbook.crons.keys(), prefix);

    // Rename entity map keys
    runbook.commands = rename_keys(std::mem::take(&mut runbook.commands), &cmd_renames);
    runbook.jobs = rename_keys(std::mem::take(&mut runbook.jobs), &job_renames);
    runbook.agents = rename_keys(std::mem::take(&mut runbook.agents), &agent_renames);
    runbook.queues = rename_keys(std::mem::take(&mut runbook.queues), &queue_renames);
    runbook.workers = rename_keys(std::mem::take(&mut runbook.workers), &worker_renames);
    runbook.crons = rename_keys(std::mem::take(&mut runbook.crons), &cron_renames);

    // Update .name fields
    update_entity_names(&mut runbook.commands, |cmd, key| cmd.name = key.to_string());
    update_entity_names(&mut runbook.jobs, |job, key| job.kind = key.to_string());
    update_entity_names(&mut runbook.agents, |agent, key| {
        agent.name = key.to_string();
    });
    update_entity_names(&mut runbook.queues, |queue, key| {
        queue.name = key.to_string();
    });
    update_entity_names(&mut runbook.workers, |worker, key| {
        worker.name = key.to_string();
    });
    update_entity_names(&mut runbook.crons, |cron, key| cron.name = key.to_string());

    // Update internal cross-references
    for worker in runbook.workers.values_mut() {
        if let Some(new) = queue_renames.get(&worker.source.queue) {
            worker.source.queue = new.clone();
        }
        if let Some(new) = job_renames.get(&worker.run.job) {
            worker.run.job = new.clone();
        }
    }

    for cron in runbook.crons.values_mut() {
        rename_run_directive(&mut cron.run, &job_renames, &agent_renames);
    }

    for cmd in runbook.commands.values_mut() {
        rename_run_directive(&mut cmd.run, &job_renames, &agent_renames);
    }

    for job in runbook.jobs.values_mut() {
        for step in &mut job.steps {
            rename_run_directive(&mut step.run, &job_renames, &agent_renames);
        }
    }
}

fn rename_keys<V>(
    map: HashMap<String, V>,
    renames: &HashMap<String, String>,
) -> HashMap<String, V> {
    map.into_iter()
        .map(|(k, v)| {
            let new_key = renames.get(&k).cloned().unwrap_or(k);
            (new_key, v)
        })
        .collect()
}

fn rename_run_directive(
    directive: &mut crate::RunDirective,
    job_renames: &HashMap<String, String>,
    agent_renames: &HashMap<String, String>,
) {
    match directive {
        crate::RunDirective::Job { job } => {
            if let Some(new) = job_renames.get(job.as_str()) {
                *job = new.clone();
            }
        }
        crate::RunDirective::Agent { agent, .. } => {
            if let Some(new) = agent_renames.get(agent.as_str()) {
                *agent = new.clone();
            }
        }
        crate::RunDirective::Shell(_) => {}
    }
}
