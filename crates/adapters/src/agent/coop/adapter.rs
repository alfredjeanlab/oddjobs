// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::http;
use crate::agent::log_entry::AgentLogMessage;
use crate::agent::{
    AgentAdapter, AgentAdapterError, AgentConfig, AgentHandle, AgentReconnectConfig,
};
use async_trait::async_trait;
use oj_core::{AgentId, AgentState, Event, OwnerId};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::Instrument;

fn default_state_dir() -> std::path::PathBuf {
    dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/state")))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("oj")
}

/// Agent adapter using coop sidecar processes.
///
/// Each agent is spawned as a coop process that wraps Claude Code in a PTY.
/// State is monitored via WebSocket subscription to coop's state stream.
/// Communication uses Unix socket HTTP.
#[derive(Clone)]
pub struct LocalAdapter {
    pub(super) state_dir: PathBuf,
    pub(super) agents: Arc<Mutex<HashMap<AgentId, CoopAgent>>>,
    pub(super) log_entry_tx: Option<mpsc::Sender<AgentLogMessage>>,
}

pub(super) struct CoopAgent {
    pub(super) socket_path: PathBuf,
    pub(super) shutdown_tx: Option<oneshot::Sender<()>>,
}

impl Default for LocalAdapter {
    fn default() -> Self {
        Self::new(default_state_dir())
    }
}

impl LocalAdapter {
    pub fn new(state_dir: PathBuf) -> Self {
        Self { state_dir, agents: Arc::new(Mutex::new(HashMap::new())), log_entry_tx: None }
    }

    /// Create a new coop agent adapter with agent log extraction
    pub fn with_log_entry_tx(mut self, tx: mpsc::Sender<AgentLogMessage>) -> Self {
        self.log_entry_tx = Some(tx);
        self
    }

    /// Get the coop socket path for a local agent.
    pub fn get_coop_socket(&self, agent_id: &AgentId) -> Option<PathBuf> {
        let agents = self.agents.lock();
        agents.get(agent_id).map(|a| a.socket_path.clone())
    }

    /// Check if a coop process is alive for the given agent_id.
    ///
    /// Used by reconciliation and IPC handlers to check agent liveness without registering.
    pub async fn check_alive(state_dir: &Path, agent_id: &str) -> bool {
        let socket_path = oj_core::agent_dir(state_dir, agent_id).join("coop.sock");
        http::get(&socket_path, "/api/v1/health").await.is_ok()
    }

    /// Resolve a pending stop attempt, allowing the next stop to proceed.
    ///
    /// Called after processing an `AgentStopBlocked` event when the engine
    /// decides the agent should be allowed to exit (e.g., after signal).
    pub async fn resolve_stop(state_dir: &Path, agent_id: &str, body: &str) {
        let socket = oj_core::agent_dir(state_dir, agent_id).join("coop.sock");
        let _ = http::post(&socket, "/api/v1/stop/resolve", body).await;
    }

    /// Kill a coop agent process by agent_id.
    ///
    /// Sends a graceful shutdown request to the coop HTTP API. Used by IPC
    /// handlers that don't have access to the registered agent map.
    pub async fn kill_agent(state_dir: &Path, agent_id: &str) {
        let socket_path = oj_core::agent_dir(state_dir, agent_id).join("coop.sock");
        let _ = http::post(&socket_path, "/api/v1/shutdown", "{}").await;
    }

    pub(super) fn register_agent(
        &self,
        agent_id: AgentId,
        socket_path: PathBuf,
        workspace_path: PathBuf,
        event_tx: mpsc::Sender<Event>,
        owner: OwnerId,
    ) -> AgentHandle {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(super::ws::event_bridge(
            socket_path.clone(),
            agent_id.clone(),
            owner,
            event_tx,
            shutdown_rx,
            self.log_entry_tx.clone(),
        ));
        self.agents
            .lock()
            .insert(agent_id.clone(), CoopAgent { socket_path, shutdown_tx: Some(shutdown_tx) });
        AgentHandle::new(agent_id, workspace_path)
    }
}

#[async_trait]
impl AgentAdapter for LocalAdapter {
    async fn spawn(
        &self,
        config: AgentConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError> {
        let span = tracing::info_span!("agent.spawn", agent_id = %config.agent_id, workspace = %config.workspace_path.display());
        async {
            let start = std::time::Instant::now();
            let result = super::spawn::execute(self, config, event_tx).await;
            let elapsed_ms = start.elapsed().as_millis() as u64;
            match &result {
                Ok(h) => tracing::info!(agent_id = %h.agent_id, elapsed_ms, "agent spawned"),
                Err(e) => tracing::error!(elapsed_ms, error = %e, "spawn failed"),
            }
            result
        }
        .instrument(span)
        .await
    }

    async fn reconnect(
        &self,
        config: AgentReconnectConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError> {
        let span = tracing::info_span!("agent.reconnect", agent_id = %config.agent_id);
        async {
            tracing::info!("reconnecting to existing session");
            let start = std::time::Instant::now();

            let socket_path =
                oj_core::agent_dir(&self.state_dir, config.agent_id.as_str()).join("coop.sock");

            // Verify coop is alive
            http::get(&socket_path, "/api/v1/health").await.map_err(|_| {
                AgentAdapterError::NotFound(format!(
                    "coop not running for agent {}",
                    config.agent_id
                ))
            })?;

            let handle = self.register_agent(
                config.agent_id,
                socket_path,
                config.workspace_path,
                event_tx,
                config.owner,
            );

            let elapsed_ms = start.elapsed().as_millis() as u64;
            tracing::info!(agent_id = %handle.agent_id, elapsed_ms, "agent reconnected");
            Ok(handle)
        }
        .instrument(span)
        .await
    }

    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError> {
        let socket_path = {
            let agents = self.agents.lock();
            agents
                .get(agent_id)
                .map(|a| a.socket_path.clone())
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?
        };

        // Use coop's nudge API (handles keyboard emulation internally)
        let body = serde_json::json!({ "message": input }).to_string();
        let response = http::post(&socket_path, "/api/v1/agent/nudge", &body).await;

        match response {
            Ok(resp) => {
                // Check if nudge was delivered
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
                    if json.get("delivered").and_then(|v| v.as_bool()) == Some(false) {
                        let reason =
                            json.get("reason").and_then(|v| v.as_str()).unwrap_or("unknown");
                        tracing::warn!(
                            %agent_id,
                            reason,
                            "nudge not delivered, falling back to raw input"
                        );
                        // Fallback: send as raw terminal input
                        let input_body =
                            serde_json::json!({ "text": input, "enter": true }).to_string();
                        http::post(&socket_path, "/api/v1/input", &input_body).await?;
                    }
                }
                Ok(())
            }
            Err(_) => {
                // Nudge endpoint may not be available, try raw input
                let input_body = serde_json::json!({ "text": input, "enter": true }).to_string();
                http::post(&socket_path, "/api/v1/input", &input_body).await?;
                Ok(())
            }
        }
    }

    async fn respond(
        &self,
        agent_id: &AgentId,
        response: &oj_core::PromptResponse,
    ) -> Result<(), AgentAdapterError> {
        let socket_path = {
            let agents = self.agents.lock();
            agents
                .get(agent_id)
                .map(|a| a.socket_path.clone())
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?
        };

        let body = serde_json::json!({
            "accept": response.accept,
            "option": response.option,
            "text": response.text,
        })
        .to_string();

        http::post(&socket_path, "/api/v1/agent/respond", &body).await?;
        Ok(())
    }

    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError> {
        tracing::info!(%agent_id, "killing agent");
        let agent = {
            self.agents
                .lock()
                .remove(agent_id)
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?
        };

        // Stop the polling task
        if let Some(tx) = agent.shutdown_tx {
            let _ = tx.send(());
        }

        // Request graceful shutdown via API
        let _ = http::post(&agent.socket_path, "/api/v1/shutdown", "{}").await;

        // Give coop time to shut down, then force via signal
        tokio::time::sleep(Duration::from_millis(500)).await;
        if http::get(&agent.socket_path, "/api/v1/health").await.is_ok() {
            let body = serde_json::json!({ "signal": "SIGKILL" }).to_string();
            let _ = http::post(&agent.socket_path, "/api/v1/signal", &body).await;
        }

        // Clean up socket file
        let _ = std::fs::remove_file(&agent.socket_path);

        Ok(())
    }

    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError> {
        let socket_path = {
            let agents = self.agents.lock();
            agents
                .get(agent_id)
                .map(|a| a.socket_path.clone())
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?
        };

        let body = http::get(&socket_path, "/api/v1/agent").await?;
        let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            AgentAdapterError::SessionError(format!("invalid JSON from coop: {}", e))
        })?;
        let state = crate::agent::parse_coop_agent_state(&json);
        tracing::trace!(%agent_id, ?state, "checked");
        Ok(state)
    }

    async fn last_message(&self, agent_id: &AgentId) -> Option<String> {
        let socket_path = {
            let agents = self.agents.lock();
            agents.get(agent_id).map(|a| a.socket_path.clone())
        }?;

        let body = http::get(&socket_path, "/api/v1/agent").await.ok()?;
        let json: serde_json::Value = serde_json::from_str(&body).ok()?;
        let msg = json.get("last_message").and_then(|v| v.as_str()).filter(|s| !s.is_empty())?;
        Some(msg.to_string())
    }

    async fn resolve_stop(&self, agent_id: &AgentId) {
        let socket_path = {
            let agents = self.agents.lock();
            match agents.get(agent_id) {
                Some(a) => a.socket_path.clone(),
                None => return,
            }
        };
        let _ = http::post(&socket_path, "/api/v1/stop/resolve", "{}").await;
    }

    async fn is_alive(&self, agent_id: &AgentId) -> bool {
        let socket_path = {
            let agents = self.agents.lock();
            match agents.get(agent_id) {
                Some(a) => a.socket_path.clone(),
                None => return false,
            }
        };

        let alive = http::get(&socket_path, "/api/v1/health").await.is_ok();
        tracing::trace!(%agent_id, alive, "checked");
        alive
    }

    async fn capture_output(
        &self,
        agent_id: &AgentId,
        _lines: u32,
    ) -> Result<String, AgentAdapterError> {
        let socket_path = {
            let agents = self.agents.lock();
            agents
                .get(agent_id)
                .map(|a| a.socket_path.clone())
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?
        };
        http::get(&socket_path, "/api/v1/screen/text").await
    }

    async fn fetch_transcript(&self, agent_id: &AgentId) -> Result<String, AgentAdapterError> {
        let socket_path = {
            let agents = self.agents.lock();
            agents
                .get(agent_id)
                .map(|a| a.socket_path.clone())
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?
        };

        // Fetch all transcripts via catchup endpoint
        let body =
            http::get(&socket_path, "/api/v1/transcripts/catchup?since_transcript=0&since_line=0")
                .await?;
        parse_transcript_response(&body)
    }

    async fn fetch_usage(&self, agent_id: &AgentId) -> Option<crate::agent::UsageData> {
        let socket_path = {
            let agents = self.agents.lock();
            agents.get(agent_id).map(|a| a.socket_path.clone())
        }?;

        let body = http::get(&socket_path, "/api/v1/session/usage").await.ok()?;
        let json: serde_json::Value = serde_json::from_str(&body).ok()?;
        Some(crate::agent::UsageData::from_json(&json))
    }
}

/// Parse a transcript catchup response into JSONL content.
pub(crate) fn parse_transcript_response(body: &str) -> Result<String, AgentAdapterError> {
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| AgentAdapterError::SessionError(format!("invalid transcript JSON: {}", e)))?;

    let mut lines = Vec::new();

    // Extract lines from completed transcripts
    if let Some(transcripts) = json.get("transcripts").and_then(|v| v.as_array()) {
        for transcript in transcripts {
            if let Some(transcript_lines) = transcript.get("lines").and_then(|v| v.as_array()) {
                for line in transcript_lines {
                    if let Some(s) = line.as_str() {
                        lines.push(s.to_string());
                    }
                }
            }
        }
    }

    // Extract lines from the live session
    if let Some(live_lines) = json.get("live_lines").and_then(|v| v.as_array()) {
        for line in live_lines {
            if let Some(s) = line.as_str() {
                lines.push(s.to_string());
            }
        }
    }

    Ok(lines.join("\n"))
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
