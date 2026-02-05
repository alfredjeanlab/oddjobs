// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Timer identifier type for tracking scheduled timers.
//!
//! TimerId uniquely identifies a timer instance used for scheduling delayed
//! actions such as timeouts, heartbeats, or periodic checks.

use crate::agent_run::AgentRunId;
use crate::job::JobId;
use crate::namespace::scoped_name;
use crate::owner::OwnerId;

crate::define_id! {
    /// Unique identifier for a timer instance.
    ///
    /// Timers are used to schedule delayed actions within the system, such as
    /// step timeouts or periodic health checks.
    pub struct TimerId;
}

impl TimerId {
    /// Timer ID for liveness monitoring of a job.
    pub fn liveness(job_id: &JobId) -> Self {
        Self::new(format!("liveness:{}", job_id))
    }

    /// Timer ID for deferred exit handling of a job.
    pub fn exit_deferred(job_id: &JobId) -> Self {
        Self::new(format!("exit-deferred:{}", job_id))
    }

    /// Timer ID for cooldown between action attempts.
    pub fn cooldown(job_id: &JobId, trigger: &str, chain_pos: usize) -> Self {
        Self::new(format!("cooldown:{}:{}:{}", job_id, trigger, chain_pos))
    }

    /// Returns true if this is a liveness timer.
    pub fn is_liveness(&self) -> bool {
        self.0.starts_with("liveness:")
    }

    /// Returns true if this is an exit-deferred timer.
    pub fn is_exit_deferred(&self) -> bool {
        self.0.starts_with("exit-deferred:")
    }

    /// Returns true if this is a cooldown timer.
    pub fn is_cooldown(&self) -> bool {
        self.0.starts_with("cooldown:")
    }

    /// Timer ID for queue item retry cooldown.
    pub fn queue_retry(queue_name: &str, item_id: &str) -> Self {
        Self::new(format!("queue-retry:{}:{}", queue_name, item_id))
    }

    /// Returns true if this is a queue retry timer.
    pub fn is_queue_retry(&self) -> bool {
        self.0.starts_with("queue-retry:")
    }

    /// Timer ID for a cron interval tick.
    pub fn cron(cron_name: &str, namespace: &str) -> Self {
        Self::new(format!("cron:{}", scoped_name(namespace, cron_name)))
    }

    /// Returns true if this is a cron timer.
    pub fn is_cron(&self) -> bool {
        self.0.starts_with("cron:")
    }

    /// Timer ID for periodic queue polling.
    pub fn queue_poll(worker_name: &str, namespace: &str) -> Self {
        Self::new(format!(
            "queue-poll:{}",
            scoped_name(namespace, worker_name)
        ))
    }

    /// Returns true if this is a queue poll timer.
    pub fn is_queue_poll(&self) -> bool {
        self.0.starts_with("queue-poll:")
    }

    /// Timer ID for liveness monitoring of a standalone agent run.
    pub fn liveness_agent_run(agent_run_id: &AgentRunId) -> Self {
        Self::new(format!("liveness:ar:{}", agent_run_id))
    }

    /// Timer ID for deferred exit handling of a standalone agent run.
    pub fn exit_deferred_agent_run(agent_run_id: &AgentRunId) -> Self {
        Self::new(format!("exit-deferred:ar:{}", agent_run_id))
    }

    /// Timer ID for cooldown between action attempts on a standalone agent run.
    pub fn cooldown_agent_run(agent_run_id: &AgentRunId, trigger: &str, chain_pos: usize) -> Self {
        Self::new(format!(
            "cooldown:ar:{}:{}:{}",
            agent_run_id, trigger, chain_pos
        ))
    }

    /// Timer ID for idle grace period before triggering on_idle for a job.
    pub fn idle_grace(job_id: &JobId) -> Self {
        Self::new(format!("idle-grace:{}", job_id))
    }

    /// Timer ID for idle grace period before triggering on_idle for a standalone agent run.
    pub fn idle_grace_agent_run(agent_run_id: &AgentRunId) -> Self {
        Self::new(format!("idle-grace:ar:{}", agent_run_id))
    }

    /// Returns true if this is an idle grace timer.
    pub fn is_idle_grace(&self) -> bool {
        self.0.starts_with("idle-grace:")
    }

    // -------------------------------------------------------------------------
    // OwnerId-based constructors (unified across Job/AgentRun owners)
    // -------------------------------------------------------------------------

    /// Timer ID for liveness monitoring, dispatching to the appropriate owner type.
    pub fn owner_liveness(owner: &OwnerId) -> Self {
        match owner {
            OwnerId::Job(job_id) => Self::liveness(job_id),
            OwnerId::AgentRun(ar_id) => Self::liveness_agent_run(ar_id),
        }
    }

    /// Timer ID for deferred exit handling, dispatching to the appropriate owner type.
    pub fn owner_exit_deferred(owner: &OwnerId) -> Self {
        match owner {
            OwnerId::Job(job_id) => Self::exit_deferred(job_id),
            OwnerId::AgentRun(ar_id) => Self::exit_deferred_agent_run(ar_id),
        }
    }

    /// Timer ID for cooldown between action attempts, dispatching to the appropriate owner type.
    pub fn owner_cooldown(owner: &OwnerId, trigger: &str, chain_pos: usize) -> Self {
        match owner {
            OwnerId::Job(job_id) => Self::cooldown(job_id, trigger, chain_pos),
            OwnerId::AgentRun(ar_id) => Self::cooldown_agent_run(ar_id, trigger, chain_pos),
        }
    }

    /// Timer ID for idle grace period, dispatching to the appropriate owner type.
    pub fn owner_idle_grace(owner: &OwnerId) -> Self {
        match owner {
            OwnerId::Job(job_id) => Self::idle_grace(job_id),
            OwnerId::AgentRun(ar_id) => Self::idle_grace_agent_run(ar_id),
        }
    }

    /// Extract the OwnerId if this timer is associated with an owner.
    pub fn owner_id(&self) -> Option<OwnerId> {
        // Check for agent_run timers first (they have :ar: marker)
        if let Some(ar_id_str) = self.agent_run_id_str() {
            return Some(OwnerId::AgentRun(AgentRunId::new(ar_id_str)));
        }
        // Then check for job timers
        if let Some(job_id_str) = self.job_id_str() {
            return Some(OwnerId::Job(JobId::new(job_id_str)));
        }
        None
    }

    /// Returns the AgentRunId portion if this is an agent-run-related timer.
    pub fn agent_run_id_str(&self) -> Option<&str> {
        if let Some(rest) = self.0.strip_prefix("liveness:ar:") {
            Some(rest)
        } else if let Some(rest) = self.0.strip_prefix("exit-deferred:ar:") {
            Some(rest)
        } else if let Some(rest) = self.0.strip_prefix("cooldown:ar:") {
            // Format: "cooldown:ar:agent_run_id:trigger:chain_pos"
            rest.split(':').next()
        } else if let Some(rest) = self.0.strip_prefix("idle-grace:ar:") {
            Some(rest)
        } else {
            None
        }
    }

    /// Returns true if this is an agent-run-related timer.
    pub fn is_agent_run_timer(&self) -> bool {
        self.agent_run_id_str().is_some()
    }

    /// Extracts the job ID portion if this is a job-related timer.
    ///
    /// Returns `Some(&str)` for liveness, exit-deferred, cooldown, and idle-grace timers.
    /// For cooldown timers, extracts the job_id from "cooldown:job_id:trigger:pos".
    ///
    /// NOTE: Returns `None` for agent_run timers (which have `:ar:` marker).
    pub fn job_id_str(&self) -> Option<&str> {
        // Agent-run timers have `:ar:` marker — exclude them
        if self.is_agent_run_timer() {
            return None;
        }

        if let Some(rest) = self.0.strip_prefix("liveness:") {
            Some(rest)
        } else if let Some(rest) = self.0.strip_prefix("exit-deferred:") {
            Some(rest)
        } else if let Some(rest) = self.0.strip_prefix("cooldown:") {
            // Format: "cooldown:job_id:trigger:chain_pos"
            rest.split(':').next()
        } else if let Some(rest) = self.0.strip_prefix("idle-grace:") {
            Some(rest)
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "timer_tests.rs"]
mod tests;
