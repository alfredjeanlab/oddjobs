// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Generic lifecycle trait for Job and Crew entities.
//!
//! `RunLifecycle` abstracts the common data accessors and entity-specific
//! effect builders needed by the unified monitor, action builder, and execution
//! code paths. Async execution stays in Runtime methods; trait methods are
//! synchronous.

use crate::RuntimeError;
use oj_core::{Crew, CrewId, CrewStatus, DecisionId, Effect, Event, Job, JobId, OwnerId, TimerId};
use oj_runbook::{AgentDef, Runbook};
use std::collections::HashMap;
use std::path::Path;

/// Common interface for entities with agent lifecycle management.
///
/// Implemented directly on `Job` and `Crew`.
pub(crate) trait RunLifecycle {
    /// The OwnerId for this entity.
    fn owner_id(&self) -> OwnerId;

    /// Human-readable name for logging and decision context.
    fn display_name(&self) -> &str;

    /// Project project.
    fn project(&self) -> &str;

    /// Current Claude agent UUID, if any.
    fn agent_id(&self) -> Option<&str>;

    /// Directory where the agent executes.
    fn execution_dir(&self) -> &Path;

    /// Variables passed to the agent.
    fn vars(&self) -> &HashMap<String, String>;

    /// Whether the entity is currently waiting for human intervention.
    fn is_waiting(&self) -> bool;

    /// Epoch ms of the last nudge sent.
    fn last_nudge_at(&self) -> Option<u64>;

    /// Returns `("job_id", id)` or `("crew_id", id)` for injecting into vars.
    fn owner_id_var(&self) -> (&str, String);

    /// Log prefix for tracing (e.g. "job_id" or "crew_id").
    fn log_id(&self) -> &str;

    /// Job step name, if applicable (None for crew).
    fn step(&self) -> Option<&str> {
        None
    }

    /// Agent reference for escalation decisions.
    fn decision_agent_ref(&self) -> String;

    /// Build the effects for auto-resuming from escalation when the agent
    /// becomes active again.
    fn auto_resume_effects(&self, resolved_at_ms: u64) -> Vec<Effect>;

    /// Build the entity-specific escalation effects (decision event is built
    /// by the caller). Includes status change events and timer cancellations.
    /// `decision_id` is passed so jobs can emit `StepWaiting` linking to the decision.
    fn escalation_status_effects(&self, trigger: &str, decision_id: Option<&str>) -> Vec<Effect>;

    /// Build a notification effect for on_done/on_fail using the entity's vars.
    fn build_notify_vars(&self, agent_def: &AgentDef) -> HashMap<String, String>;

    /// Content hash of the stored runbook for this entity.
    fn runbook_hash(&self) -> &str;

    /// Resolve the agent definition from the runbook for this entity.
    fn resolve_agent_def(&self, runbook: &Runbook) -> Result<AgentDef, RuntimeError>;
}

impl RunLifecycle for Job {
    fn owner_id(&self) -> OwnerId {
        JobId::new(&self.id).into()
    }

    fn display_name(&self) -> &str {
        &self.name
    }

    fn project(&self) -> &str {
        &self.project
    }

    fn agent_id(&self) -> Option<&str> {
        self.step_history.iter().rfind(|r| r.name == self.step).and_then(|r| r.agent_id.as_deref())
    }

    fn execution_dir(&self) -> &Path {
        Job::execution_dir(self)
    }

    fn vars(&self) -> &HashMap<String, String> {
        &self.vars
    }

    fn is_waiting(&self) -> bool {
        self.step_status.is_waiting()
    }

    fn last_nudge_at(&self) -> Option<u64> {
        self.last_nudge_at
    }

    fn owner_id_var(&self) -> (&str, String) {
        ("job_id", self.id.clone())
    }

    fn log_id(&self) -> &str {
        &self.id
    }

    fn step(&self) -> Option<&str> {
        Some(&self.step)
    }

    fn decision_agent_ref(&self) -> String {
        self.agent_id().unwrap_or_default().to_string()
    }

    fn auto_resume_effects(&self, resolved_at_ms: u64) -> Vec<Effect> {
        let job_id = JobId::new(&self.id);
        let mut effects = vec![Effect::Emit {
            event: Event::StepStarted {
                job_id: job_id.clone(),
                step: self.step.clone(),
                agent_id: None,
                agent_name: None,
            },
        }];

        if let oj_core::StepStatus::Waiting(Some(ref decision_id)) = self.step_status {
            effects.push(Effect::Emit {
                event: Event::DecisionResolved {
                    id: DecisionId::new(decision_id.clone()),
                    choices: vec![],
                    message: Some("auto-dismissed: agent became active".to_string()),
                    resolved_at_ms,
                    project: self.project.clone(),
                },
            });
        }

        effects
    }

    fn escalation_status_effects(&self, trigger: &str, decision_id: Option<&str>) -> Vec<Effect> {
        let job_id = JobId::new(&self.id);
        vec![
            Effect::Emit {
                event: Event::StepWaiting {
                    job_id: job_id.clone(),
                    step: self.step.clone(),
                    reason: Some(trigger.to_string()),
                    decision_id: decision_id.map(|s| s.to_string()),
                },
            },
            Effect::CancelTimer { id: TimerId::exit_deferred(&job_id) },
        ]
    }

    fn build_notify_vars(&self, agent_def: &AgentDef) -> HashMap<String, String> {
        let mut vars = crate::vars::namespace_vars(&self.vars);
        vars.insert("job_id".to_string(), self.id.clone());
        vars.insert("name".to_string(), self.name.clone());
        vars.insert("agent".to_string(), agent_def.name.clone());
        vars.insert("step".to_string(), self.step.clone());
        if let Some(err) = &self.error {
            vars.insert("error".to_string(), err.clone());
        }
        vars
    }

    fn runbook_hash(&self) -> &str {
        &self.runbook_hash
    }

    fn resolve_agent_def(&self, runbook: &Runbook) -> Result<AgentDef, RuntimeError> {
        crate::monitor::get_agent_def(runbook, self).cloned()
    }
}

impl RunLifecycle for Crew {
    fn owner_id(&self) -> OwnerId {
        CrewId::new(&self.id).into()
    }

    fn display_name(&self) -> &str {
        &self.command_name
    }

    fn project(&self) -> &str {
        &self.project
    }

    fn agent_id(&self) -> Option<&str> {
        self.agent_id.as_deref()
    }

    fn execution_dir(&self) -> &Path {
        &self.cwd
    }

    fn vars(&self) -> &HashMap<String, String> {
        &self.vars
    }

    fn is_waiting(&self) -> bool {
        self.status == CrewStatus::Escalated || self.status == CrewStatus::Waiting
    }

    fn last_nudge_at(&self) -> Option<u64> {
        self.last_nudge_at
    }

    fn owner_id_var(&self) -> (&str, String) {
        ("crew_id", self.id.clone())
    }

    fn log_id(&self) -> &str {
        &self.id
    }

    fn decision_agent_ref(&self) -> String {
        self.agent_id.clone().unwrap_or_default()
    }

    fn auto_resume_effects(&self, resolved_at_ms: u64) -> Vec<Effect> {
        let crew_id = CrewId::new(&self.id);
        let effects = vec![Effect::Emit {
            event: Event::CrewUpdated {
                id: crew_id.clone(),
                status: CrewStatus::Running,
                reason: Some("agent active".to_string()),
            },
        }];

        // Auto-dismiss pending decision for this crew.
        // Unlike jobs which store the decision_id in StepStatus::Waiting,
        // crew find the pending decision via owner lookup.
        // The caller (handle_monitor_state_for) handles the decision
        // lookup and appends the DecisionResolved effect.
        let _ = resolved_at_ms; // Used by caller for decision resolution

        effects
    }

    fn escalation_status_effects(&self, trigger: &str, _decision_id: Option<&str>) -> Vec<Effect> {
        let crew_id = CrewId::new(&self.id);
        vec![
            Effect::Emit {
                event: Event::CrewUpdated {
                    id: crew_id.clone(),
                    status: CrewStatus::Escalated,
                    reason: Some(trigger.to_string()),
                },
            },
            Effect::CancelTimer { id: TimerId::exit_deferred(&crew_id) },
        ]
    }

    fn build_notify_vars(&self, agent_def: &AgentDef) -> HashMap<String, String> {
        let mut vars = crate::vars::namespace_vars(&self.vars);
        vars.insert("crew_id".to_string(), self.id.clone());
        vars.insert("name".to_string(), self.command_name.clone());
        vars.insert("agent".to_string(), agent_def.name.clone());
        if let Some(err) = &self.error {
            vars.insert("error".to_string(), err.clone());
        }
        vars
    }

    fn runbook_hash(&self) -> &str {
        &self.runbook_hash
    }

    fn resolve_agent_def(&self, runbook: &Runbook) -> Result<AgentDef, RuntimeError> {
        runbook
            .get_agent(&self.agent_name)
            .cloned()
            .ok_or_else(|| RuntimeError::AgentNotFound(self.agent_name.clone()))
    }
}

/// Build an on_start notification effect.
pub(crate) fn notify_on_start(run: &dyn RunLifecycle, agent_def: &AgentDef) -> Option<Effect> {
    build_notify_effect(run, agent_def, agent_def.notify.on_start.as_ref())
}

/// Build an on_done notification effect.
pub(crate) fn notify_on_done(run: &dyn RunLifecycle, agent_def: &AgentDef) -> Option<Effect> {
    build_notify_effect(run, agent_def, agent_def.notify.on_done.as_ref())
}

fn build_notify_effect(
    run: &dyn RunLifecycle,
    agent_def: &AgentDef,
    message_template: Option<&String>,
) -> Option<Effect> {
    let template = message_template?;
    let vars = run.build_notify_vars(agent_def);
    let message = oj_runbook::NotifyConfig::render(template, &vars);
    Some(Effect::Notify { title: agent_def.name.clone(), message })
}
