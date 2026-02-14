// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job and step event handlers.

use oj_core::{AgentRecordStatus, Event, Job, JobConfig, OwnerId, StepOutcome, StepStatus};

use super::helpers;
use super::types::StoredRunbook;
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::JobCreated {
            id,
            kind,
            name,
            runbook_hash,
            cwd,
            vars,
            initial_step,
            created_at_ms,
            project,
            cron,
        } => {
            let mut builder =
                JobConfig::builder(id.to_string(), kind.clone(), initial_step.clone())
                    .name(name.clone())
                    .vars(vars.clone())
                    .runbook_hash(runbook_hash.clone())
                    .cwd(cwd.clone())
                    .project(project.clone());
            if let Some(cn) = cron {
                builder = builder.cron_name(cn.clone());
            }
            let config = builder.build();
            let job = Job::new_with_epoch_ms(config, *created_at_ms);
            state.jobs.insert(id.to_string(), job);
        }

        Event::RunbookLoaded { hash, version, runbook } => {
            // Only insert if not already present (dedup by content hash)
            if !state.runbooks.contains_key(hash) {
                state.runbooks.insert(
                    hash.clone(),
                    StoredRunbook { version: *version, data: runbook.clone() },
                );
            }
        }

        Event::JobAdvanced { id, step } => {
            if let Some(job) = state.jobs.get_mut(id.as_str()) {
                // Idempotency: skip if already on this step, UNLESS recovering
                // from failure (on_fail → same step cycle).
                let is_failure_transition = job.step_status == StepStatus::Failed;
                if job.step == *step && !is_failure_transition {
                    return;
                }
                // Clear stale error and session when resuming from terminal state
                let was_terminal = job.is_terminal();
                let target_is_nonterminal = step != "done"
                    && step != "failed"
                    && step != "cancelled"
                    && step != "suspended";
                if was_terminal && target_is_nonterminal {
                    job.error = None;
                    job.suspending = false;
                }

                let now = helpers::epoch_ms_now();
                // Finalize the previous step
                let outcome = match step.as_str() {
                    "failed" | "cancelled" | "suspended" => {
                        StepOutcome::Failed(job.error.clone().unwrap_or_default())
                    }
                    _ => StepOutcome::Completed,
                };
                job.finalize_current_step(outcome, now);

                job.step = step.clone();
                job.step_status = match step.as_str() {
                    "failed" | "cancelled" => StepStatus::Failed,
                    "suspended" => StepStatus::Suspended,
                    "done" => StepStatus::Completed,
                    _ => StepStatus::Pending,
                };

                // Only reset action attempts on success transitions.
                // On failure (on_fail) transitions, preserve attempts so that
                // cycle limits work — the agent action's `attempts` field should
                // bound retries across the entire on_fail chain, not per-step.
                if !is_failure_transition {
                    job.actions.reset_attempts();
                    // Clear failing flag on success transitions: when a cleanup
                    // step (routed via on_fail) completes and its on_done
                    // advances to a new step, the recovery chain succeeded.
                    // Without this, the failing flag persists and causes the
                    // job to terminate as failed when the next step completes.
                    job.failing = false;
                }
                // Push new step record and track visits (unless terminal)
                if step != "done" && step != "failed" && step != "cancelled" && step != "suspended"
                {
                    job.record_step_visit(step);
                    job.push_step(step, now);
                }
            }

            // Remove from worker active_job_ids and item_owners on terminal states
            if step == "done" || step == "failed" || step == "cancelled" || step == "suspended" {
                let job_id_str = id.to_string();
                for record in state.workers.values_mut() {
                    record.active.retain(|pid| pid != &job_id_str);
                    record.owners.remove(&job_id_str);
                }
                // Clean up unresolved decisions for the completed job
                helpers::cleanup_unresolved_decisions_for_owner(
                    &mut state.decisions,
                    &(*id).into(),
                );
            }
        }

        Event::StepStarted { job_id, agent_id, agent_name, .. } => {
            if let Some(job) = state.jobs.get_mut(job_id.as_str()) {
                job.step_status = StepStatus::Running;
                if let Some(aid) = agent_id {
                    job.set_current_step_agent_id(aid.as_str());

                    // Insert unified agent record for job-embedded agents
                    let workspace =
                        job.workspace_path.as_ref().cloned().unwrap_or_else(|| job.cwd.clone());
                    state.agents.entry(aid.as_str().to_string()).or_insert_with(|| {
                        helpers::create_agent_record(
                            aid.as_str(),
                            agent_name.clone().unwrap_or_default(),
                            (*job_id).into(),
                            job.project.clone(),
                            workspace,
                            AgentRecordStatus::Starting,
                        )
                    });
                }
                if let Some(aname) = agent_name {
                    job.set_current_step_agent_name(aname.as_str());
                }
                job.update_current_step_outcome(StepOutcome::Running);
            }
        }

        Event::StepWaiting { job_id, reason, decision_id, .. } => {
            if let Some(job) = state.jobs.get_mut(job_id.as_str()) {
                job.step_status = StepStatus::Waiting(decision_id.clone());
                if reason.is_some() {
                    job.error.clone_from(reason);
                }
                let reason_str = reason.clone().unwrap_or_default();
                job.update_current_step_outcome(StepOutcome::Waiting(reason_str));
            }
        }

        Event::StepCompleted { job_id, .. } => {
            if let Some(job) = state.jobs.get_mut(job_id.as_str()) {
                job.step_status = StepStatus::Completed;
                job.finalize_current_step(StepOutcome::Completed, helpers::epoch_ms_now());
            }
        }

        Event::StepFailed { job_id, error, .. } => {
            if let Some(job) = state.jobs.get_mut(job_id.as_str()) {
                // Don't overwrite Suspended status — suspension_effects() emits
                // JobAdvanced{step:"suspended"} then StepFailed, and the latter
                // must not reset the status back to Failed.
                if !job.step_status.is_suspended() {
                    job.step_status = StepStatus::Failed;
                }
                job.error = Some(error.clone());
                job.finalize_current_step(
                    StepOutcome::Failed(error.clone()),
                    helpers::epoch_ms_now(),
                );
            }
        }

        Event::JobFailing { id } => {
            if let Some(job) = state.jobs.get_mut(id.as_str()) {
                job.failing = true;
            }
        }

        Event::JobCancelling { id } => {
            if let Some(job) = state.jobs.get_mut(id.as_str()) {
                job.cancelling = true;
            }
        }

        Event::JobSuspending { id } => {
            if let Some(job) = state.jobs.get_mut(id.as_str()) {
                job.suspending = true;
            }
        }

        Event::JobDeleted { id } => {
            state.jobs.remove(id.as_str());
            // Clean up all decisions and agents associated with the deleted job
            let owner = OwnerId::Job(*id);
            state.decisions.retain(|_, d| d.owner != owner);
            state.agents.retain(|_, rec| rec.owner != owner);
        }

        Event::ShellExited { job_id, exit_code, .. } => {
            if let Some(job) = state.jobs.get_mut(job_id.as_str()) {
                let now = helpers::epoch_ms_now();
                if *exit_code == 0 {
                    job.step_status = StepStatus::Completed;
                    job.finalize_current_step(StepOutcome::Completed, now);
                } else {
                    let error_msg = format!("shell exit code: {}", exit_code);
                    job.step_status = StepStatus::Failed;
                    job.error = Some(error_msg.clone());
                    job.finalize_current_step(StepOutcome::Failed(error_msg), now);
                }
            }
        }

        Event::JobUpdated { id, vars } => {
            if let Some(job) = state.jobs.get_mut(id.as_str()) {
                for (key, value) in vars {
                    job.vars.insert(key.clone(), value.clone());
                }
            }
        }

        _ => {}
    }
}
