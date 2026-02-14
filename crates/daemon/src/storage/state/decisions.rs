// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision, crew, and command event handlers.

use oj_core::{AgentRecordStatus, Crew, CrewStatus, Decision, Event, OwnerId, StepStatus};

use super::helpers;
use super::MaterializedState;

pub(crate) fn apply(state: &mut MaterializedState, event: &Event) {
    match event {
        Event::DecisionCreated {
            id,
            agent_id,
            owner,
            source,
            context,
            options,
            questions,
            created_at_ms,
            project,
        } => {
            // Idempotency: skip if already exists
            if !state.decisions.contains_key(id.as_str()) {
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
                for existing in state.decisions.values_mut() {
                    if existing.owner == *owner && !existing.is_resolved() {
                        existing.resolved_at_ms = Some(*created_at_ms);
                        existing.superseded_by = Some(*id);
                    }
                }

                state.decisions.insert(
                    id.to_string(),
                    Decision {
                        id: *id,
                        agent_id: *agent_id,
                        owner: *owner,
                        source: source.clone(),
                        context: context.clone(),
                        options: options.clone(),
                        questions: questions.clone(),
                        choices: Vec::new(),
                        message: None,
                        created_at_ms: *created_at_ms,
                        resolved_at_ms: None,
                        superseded_by: None,
                        project: project.clone(),
                    },
                );
            }

            // Route by owner for setting status
            match owner {
                OwnerId::Job(job_id) => {
                    if let Some(job) = state.jobs.get_mut(job_id.as_str()) {
                        job.step_status = StepStatus::Waiting(Some(id.to_string()));
                    }
                }
                OwnerId::Crew(crew_id) => {
                    if let Some(crew) = state.crew.get_mut(crew_id.as_str()) {
                        crew.status = CrewStatus::Waiting;
                    }
                }
            }
        }

        Event::DecisionResolved { id, choices, message, resolved_at_ms, .. } => {
            if let Some(decision) = state.decisions.get_mut(id.as_str()) {
                decision.choices.clone_from(choices);
                decision.message.clone_from(message);
                decision.resolved_at_ms = Some(*resolved_at_ms);
            }
        }

        Event::CrewCreated {
            id,
            agent,
            command,
            project,
            cwd,
            runbook_hash,
            vars,
            created_at_ms,
        } => {
            state.crew.insert(
                id.as_str().to_string(),
                Crew {
                    id: id.as_str().to_string(),
                    agent_name: agent.clone(),
                    command_name: command.clone(),
                    project: project.clone(),
                    cwd: cwd.clone(),
                    runbook_hash: runbook_hash.clone(),
                    status: CrewStatus::Starting,
                    agent_id: None,
                    error: None,
                    created_at_ms: *created_at_ms,
                    updated_at_ms: *created_at_ms,
                    actions: Default::default(),
                    vars: vars.clone(),
                    last_nudge_at: None,
                },
            );
        }

        Event::CrewStarted { id, agent_id } => {
            if let Some(run) = state.crew.get_mut(id.as_str()) {
                run.status = CrewStatus::Running;
                run.agent_id = Some(agent_id.as_str().to_string());
                run.updated_at_ms = helpers::epoch_ms_now();

                // Insert unified agent record for standalone agents
                state.agents.entry(agent_id.as_str().to_string()).or_insert_with(|| {
                    helpers::create_agent_record(
                        agent_id.as_str(),
                        run.agent_name.clone(),
                        (*id).into(),
                        run.project.clone(),
                        run.cwd.clone(),
                        AgentRecordStatus::Running,
                    )
                });
            }
        }

        Event::CrewUpdated { id, status, reason } => {
            if let Some(run) = state.crew.get_mut(id.as_str()) {
                run.status = status.clone();
                if let Some(reason) = reason {
                    run.error = Some(reason.clone());
                }
                run.updated_at_ms = helpers::epoch_ms_now();
            }

            // Clean up unresolved decisions for terminal crew
            if status.is_terminal() {
                helpers::cleanup_unresolved_decisions_for_owner(
                    &mut state.decisions,
                    &(*id).into(),
                );
            }
        }

        Event::CrewDeleted { id } => {
            state.crew.remove(id.as_str());
            // Remove agents owned by this crew
            let owner = OwnerId::Crew(*id);
            state.agents.retain(|_, rec| rec.owner != owner);
        }

        // CommandRun: only persist the project → project_path mapping
        Event::CommandRun { project, project_path, .. } => {
            state.project_paths.insert(project.clone(), project_path.clone());
        }

        _ => {}
    }
}
