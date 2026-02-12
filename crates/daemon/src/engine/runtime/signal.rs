// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent terminal capture and session transcript archival.

use super::Runtime;
use crate::adapters::{AgentAdapter, NotifyAdapter};
use oj_core::{AgentId, Clock, Crew, Job};

impl<A, N, C> Runtime<A, N, C>
where
    A: AgentAdapter,
    N: NotifyAdapter,
    C: Clock,
{
    /// Capture agent terminal output and save to the agent's log directory.
    ///
    /// Best-effort: failures are logged but do not interrupt signal handling.
    pub(crate) async fn capture_agent_terminal(&self, agent_id: &AgentId) {
        if let Ok(output) = self.executor.capture_agent_output(agent_id, 200).await {
            self.logger.write_agent_capture(agent_id.as_str(), &output);
        }
    }

    /// Archive an agent's session transcript to the logs directory.
    ///
    /// Fetches the transcript from coop's API and writes it to
    /// `{logs}/agent/{agent_id}/session.jsonl` for archival.
    pub(crate) async fn archive_session_transcript(&self, agent_id: &AgentId) {
        match self.executor.fetch_transcript(agent_id).await {
            Ok(content) if !content.is_empty() => {
                self.logger.write_session_log(agent_id.as_str(), &content);
            }
            _ => {}
        }
    }

    /// Best-effort capture of terminal output and session log before killing a job's agent.
    pub(crate) async fn capture_before_kill_job(&self, job: &Job) {
        let Some(agent_id) = super::monitor::step_agent_id(job) else {
            return;
        };
        let agent_id = AgentId::new(agent_id);
        self.capture_agent_terminal(&agent_id).await;
        self.archive_session_transcript(&agent_id).await;
    }

    /// Best-effort capture of terminal output and session log before killing a standalone agent.
    pub(crate) async fn capture_before_kill_crew(&self, crew: &Crew) {
        let Some(ref agent_id) = crew.agent_id else {
            return;
        };
        let agent_id = AgentId::new(agent_id);
        self.capture_agent_terminal(&agent_id).await;
        self.archive_session_transcript(&agent_id).await;
    }
}
