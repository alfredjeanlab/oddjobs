// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision, agent run, and command event handlers.

use oj_core::{
    AgentRecordStatus, AgentRun, AgentRunStatus, Decision, DecisionId, Event, OwnerId, StepStatus,
};

use super::helpers;
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::DecisionCreated {
            id,
            job_id,
            agent_id,
            owner,
            source,
            context,
            options,
            question_data,
            created_at_ms,
            namespace,
        } => {
            // Idempotency: skip if already exists
            if !state.decisions.contains_key(id) {
                // Don't create if a more-specific decision already exists for this owner.
                // Prevents a generic `permission_prompt` (Approval) from overriding
                // a more-specific AskUserQuestion (Question) or ExitPlanMode (Plan).
                let dominated = state.decisions.values().any(|d| {
                    d.owner == *owner && !d.is_resolved() && !source.should_supersede(&d.source)
                });
                if dominated {
                    return;
                }

                // Auto-dismiss previous unresolved decisions for the same owner
                let new_decision_id = DecisionId::new(id.clone());
                for existing in state.decisions.values_mut() {
                    if existing.owner == *owner && !existing.is_resolved() {
                        existing.resolved_at_ms = Some(*created_at_ms);
                        existing.superseded_by = Some(new_decision_id.clone());
                    }
                }

                state.decisions.insert(
                    id.clone(),
                    Decision {
                        id: new_decision_id,
                        job_id: job_id.to_string(),
                        agent_id: agent_id.clone(),
                        owner: owner.clone(),
                        source: source.clone(),
                        context: context.clone(),
                        options: options.clone(),
                        question_data: question_data.clone(),
                        chosen: None,
                        choices: Vec::new(),
                        message: None,
                        created_at_ms: *created_at_ms,
                        resolved_at_ms: None,
                        superseded_by: None,
                        namespace: namespace.clone(),
                    },
                );
            }

            // Route by owner for setting status
            match owner {
                OwnerId::Job(jid) => {
                    if let Some(job) = state.jobs.get_mut(jid.as_str()) {
                        job.step_status = StepStatus::Waiting(Some(id.clone()));
                    }
                }
                OwnerId::AgentRun(ar_id) => {
                    if let Some(agent_run) = state.agent_runs.get_mut(ar_id.as_str()) {
                        agent_run.status = AgentRunStatus::Waiting;
                    }
                }
            }
        }

        Event::DecisionResolved {
            id,
            chosen,
            choices,
            message,
            resolved_at_ms,
            ..
        } => {
            if let Some(decision) = state.decisions.get_mut(id) {
                decision.chosen = *chosen;
                decision.choices.clone_from(choices);
                decision.message.clone_from(message);
                decision.resolved_at_ms = Some(*resolved_at_ms);
            }
        }

        Event::AgentRunCreated {
            id,
            agent_name,
            command_name,
            namespace,
            cwd,
            runbook_hash,
            vars,
            created_at_epoch_ms,
        } => {
            state.agent_runs.insert(
                id.as_str().to_string(),
                AgentRun {
                    id: id.as_str().to_string(),
                    agent_name: agent_name.clone(),
                    command_name: command_name.clone(),
                    namespace: namespace.clone(),
                    cwd: cwd.clone(),
                    runbook_hash: runbook_hash.clone(),
                    status: AgentRunStatus::Starting,
                    agent_id: None,
                    session_id: None,
                    error: None,
                    created_at_ms: *created_at_epoch_ms,
                    updated_at_ms: *created_at_epoch_ms,
                    action_tracker: Default::default(),
                    vars: vars.clone(),
                    idle_grace_log_size: None,
                    last_nudge_at: None,
                },
            );
        }

        Event::AgentRunStarted { id, agent_id } => {
            if let Some(run) = state.agent_runs.get_mut(id.as_str()) {
                run.status = AgentRunStatus::Running;
                run.agent_id = Some(agent_id.as_str().to_string());
                run.updated_at_ms = helpers::epoch_ms_now();

                // Insert unified agent record for standalone agents
                state
                    .agents
                    .entry(agent_id.as_str().to_string())
                    .or_insert_with(|| {
                        helpers::create_agent_record(
                            agent_id.as_str(),
                            run.agent_name.clone(),
                            OwnerId::AgentRun(id.clone()),
                            run.namespace.clone(),
                            run.cwd.clone(),
                            AgentRecordStatus::Running,
                        )
                    });
            }
        }

        Event::AgentRunStatusChanged { id, status, reason } => {
            if let Some(run) = state.agent_runs.get_mut(id.as_str()) {
                run.status = status.clone();
                if let Some(reason) = reason {
                    run.error = Some(reason.clone());
                }
                run.updated_at_ms = helpers::epoch_ms_now();
            }

            // Clean up unresolved decisions for terminal agent runs
            if status.is_terminal() {
                helpers::cleanup_unresolved_decisions_for_owner(
                    &mut state.decisions,
                    &OwnerId::AgentRun(id.clone()),
                );
            }
        }

        Event::AgentRunDeleted { id } => {
            state.agent_runs.remove(id.as_str());
            // Remove agents owned by this agent_run
            let owner = OwnerId::AgentRun(id.clone());
            state.agents.retain(|_, rec| rec.owner != owner);
        }

        // CommandRun: only persist the namespace → project_root mapping
        Event::CommandRun {
            namespace,
            project_root,
            ..
        } => {
            if !namespace.is_empty() {
                state
                    .project_roots
                    .insert(namespace.clone(), project_root.clone());
            }
        }

        _ => {}
    }
}
