// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Traced adapter wrappers for consistent observability

use crate::agent::{AgentAdapter, AgentError, AgentHandle, AgentReconnectConfig, AgentSpawnConfig};
use crate::session::{SessionAdapter, SessionError};
use async_trait::async_trait;
use oj_core::{AgentId, AgentState, Event};
use std::path::Path;
use tokio::sync::mpsc;

/// Wrapper that adds tracing to any SessionAdapter
#[derive(Clone)]
pub struct TracedSession<S> {
    inner: S,
}

impl<S> TracedSession<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<S: SessionAdapter> SessionAdapter for TracedSession<S> {
    async fn spawn(
        &self,
        name: &str,
        cwd: &Path,
        cmd: &str,
        env: &[(String, String)],
    ) -> Result<String, SessionError> {
        let span = tracing::info_span!("session.spawn", name, cwd = %cwd.display());
        let _guard = span.enter();

        tracing::info!(cmd, env_count = env.len(), "starting");

        let start = std::time::Instant::now();
        let result = self.inner.spawn(name, cwd, cmd, env).await;
        let elapsed = start.elapsed();

        match &result {
            Ok(session_id) => tracing::info!(
                session_id,
                elapsed_ms = elapsed.as_millis() as u64,
                "session created"
            ),
            Err(e) => tracing::error!(
                elapsed_ms = elapsed.as_millis() as u64,
                error = %e,
                "spawn failed"
            ),
        }

        result
    }

    async fn send(&self, id: &str, input: &str) -> Result<(), SessionError> {
        let span = tracing::info_span!("session.send", id);
        let _guard = span.enter();

        tracing::debug!(input_len = input.len(), "sending");
        let result = self.inner.send(id, input).await;

        match &result {
            Ok(()) => tracing::debug!("sent"),
            Err(e) => tracing::error!(error = %e, "send failed"),
        }

        result
    }

    async fn send_literal(&self, id: &str, text: &str) -> Result<(), SessionError> {
        let span = tracing::info_span!("session.send_literal", id);
        let _guard = span.enter();

        tracing::debug!(text_len = text.len(), "sending literal");
        let result = self.inner.send_literal(id, text).await;

        match &result {
            Ok(()) => tracing::debug!("sent literal"),
            Err(e) => tracing::error!(error = %e, "send_literal failed"),
        }

        result
    }

    async fn send_enter(&self, id: &str) -> Result<(), SessionError> {
        let span = tracing::info_span!("session.send_enter", id);
        let _guard = span.enter();

        tracing::debug!("sending enter");
        let result = self.inner.send_enter(id).await;

        match &result {
            Ok(()) => tracing::debug!("sent enter"),
            Err(e) => tracing::error!(error = %e, "send_enter failed"),
        }

        result
    }

    async fn kill(&self, id: &str) -> Result<(), SessionError> {
        let span = tracing::info_span!("session.kill", id);
        let _guard = span.enter();

        let result = self.inner.kill(id).await;
        // kill() failing is often acceptable (session already gone)
        match &result {
            Ok(()) => tracing::info!("killed"),
            Err(e) => tracing::warn!(error = %e, "kill failed (may be expected)"),
        }

        result
    }

    async fn is_alive(&self, id: &str) -> Result<bool, SessionError> {
        let result = self.inner.is_alive(id).await;
        tracing::trace!(id, alive = ?result.as_ref().ok(), "checked");
        result
    }

    async fn capture_output(&self, id: &str, lines: u32) -> Result<String, SessionError> {
        let span = tracing::info_span!("session.capture", id, lines);
        let _guard = span.enter();

        let result = self.inner.capture_output(id, lines).await;
        tracing::debug!(
            captured_len = result.as_ref().map(|s| s.len()).ok(),
            "captured"
        );
        result
    }

    async fn is_process_running(&self, id: &str, pattern: &str) -> Result<bool, SessionError> {
        self.inner.is_process_running(id, pattern).await
    }

    async fn get_exit_code(&self, id: &str) -> Result<Option<i32>, SessionError> {
        self.inner.get_exit_code(id).await
    }
}

/// Wrapper that adds tracing to any AgentAdapter
#[derive(Clone)]
pub struct TracedAgent<A> {
    inner: A,
}

impl<A> TracedAgent<A> {
    pub fn new(inner: A) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<A: AgentAdapter> AgentAdapter for TracedAgent<A> {
    async fn spawn(
        &self,
        config: AgentSpawnConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentError> {
        let span = tracing::info_span!(
            "agent.spawn",
            agent_id = %config.agent_id,
            workspace = %config.workspace_path.display(),
        );
        let _guard = span.enter();

        tracing::info!(command = %config.command, "starting");

        let start = std::time::Instant::now();
        let result = self.inner.spawn(config, event_tx).await;
        let elapsed = start.elapsed();

        match &result {
            Ok(handle) => tracing::info!(
                agent_id = %handle.agent_id,
                elapsed_ms = elapsed.as_millis() as u64,
                "agent spawned"
            ),
            Err(e) => tracing::error!(
                elapsed_ms = elapsed.as_millis() as u64,
                error = %e,
                "spawn failed"
            ),
        }

        result
    }

    async fn reconnect(
        &self,
        config: AgentReconnectConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentError> {
        let span = tracing::info_span!(
            "agent.reconnect",
            agent_id = %config.agent_id,
            session_id = %config.session_id,
        );
        let _guard = span.enter();

        tracing::info!("reconnecting to existing session");

        let start = std::time::Instant::now();
        let result = self.inner.reconnect(config, event_tx).await;
        let elapsed = start.elapsed();

        match &result {
            Ok(handle) => tracing::info!(
                agent_id = %handle.agent_id,
                elapsed_ms = elapsed.as_millis() as u64,
                "agent reconnected"
            ),
            Err(e) => tracing::error!(
                elapsed_ms = elapsed.as_millis() as u64,
                error = %e,
                "reconnect failed"
            ),
        }

        result
    }

    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentError> {
        let span = tracing::info_span!("agent.send", %agent_id);
        let _guard = span.enter();

        tracing::debug!(input_len = input.len(), "sending");
        let result = self.inner.send(agent_id, input).await;

        match &result {
            Ok(()) => tracing::debug!("sent"),
            Err(e) => tracing::error!(error = %e, "send failed"),
        }

        result
    }

    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentError> {
        let span = tracing::info_span!("agent.kill", %agent_id);
        let _guard = span.enter();

        let result = self.inner.kill(agent_id).await;
        // kill() failing is often acceptable (agent already gone)
        match &result {
            Ok(()) => tracing::info!("killed"),
            Err(e) => tracing::warn!(error = %e, "kill failed (may be expected)"),
        }

        result
    }

    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentError> {
        let result = self.inner.get_state(agent_id).await;
        tracing::trace!(%agent_id, state = ?result.as_ref().ok(), "checked");
        result
    }

    async fn session_log_size(&self, agent_id: &AgentId) -> Option<u64> {
        self.inner.session_log_size(agent_id).await
    }
}

#[cfg(test)]
#[path = "traced_tests.rs"]
mod tests;
