// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared remote-coop client for Docker and Kubernetes adapters.
//!
//! Both adapters communicate with coop over TCP HTTP using the same protocol
//! (bearer-token auth, identical API calls). This module extracts the common
//! agent registry and the 9 `AgentAdapter` methods that are transport-agnostic
//! once an agent has been registered.

use crate::adapters::agent::coop::adapter::parse_transcript_response;
use crate::adapters::agent::docker::http;
use crate::adapters::agent::log_entry::AgentLogMessage;
use crate::adapters::agent::{AgentAdapterError, AgentHandle, UsageData};
use oj_core::{AgentId, AgentState, Event, OwnerId};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// Per-agent state tracked by the remote client.
struct RemoteAgent {
    /// Host address for TCP communication (e.g., "127.0.0.1:9001" or "10.0.1.5:8080").
    addr: String,
    /// Bearer token for authenticating with coop.
    auth_token: String,
    /// Shutdown sender for the WS event bridge. Dropping this closes the bridge.
    bridge_shutdown: Option<oneshot::Sender<()>>,
    /// Event channel for the bridge to emit events. Stored for bridge reconnection.
    event_tx: mpsc::Sender<Event>,
    /// Owner of this agent. Stored for bridge reconnection.
    owner: OwnerId,
}

/// Shared registry and API client for remote (TCP) coop agents.
///
/// Used by both `DockerAdapter` and `KubernetesAdapter` to avoid duplicating
/// the 9 trait methods that operate identically once the agent is registered.
#[derive(Clone)]
pub(crate) struct RemoteCoopClient {
    agents: Arc<Mutex<HashMap<AgentId, RemoteAgent>>>,
    log_entry_tx: Option<mpsc::Sender<AgentLogMessage>>,
}

impl RemoteCoopClient {
    pub(crate) fn new() -> Self {
        Self { agents: Arc::new(Mutex::new(HashMap::new())), log_entry_tx: None }
    }

    pub(crate) fn with_log_entry_tx(mut self, tx: mpsc::Sender<AgentLogMessage>) -> Self {
        self.log_entry_tx = Some(tx);
        self
    }

    /// Get the TCP address and auth token for a registered agent.
    pub(crate) fn get_coop_host(&self, agent_id: &AgentId) -> Option<(String, String)> {
        let agents = self.agents.lock();
        agents.get(agent_id).map(|a| (a.addr.clone(), a.auth_token.clone()))
    }

    /// Look up an agent's (addr, token), returning `NotFound` if missing.
    fn lookup(&self, agent_id: &AgentId) -> Result<(String, String), AgentAdapterError> {
        let agents = self.agents.lock();
        let agent = agents
            .get(agent_id)
            .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?;
        Ok((agent.addr.clone(), agent.auth_token.clone()))
    }

    /// Look up an agent's (addr, token), returning `None` if missing.
    fn lookup_opt(&self, agent_id: &AgentId) -> Option<(String, String)> {
        let agents = self.agents.lock();
        let agent = agents.get(agent_id)?;
        Some((agent.addr.clone(), agent.auth_token.clone()))
    }

    /// Update the address for a registered agent (e.g., after pod IP change)
    /// and restart the WS event bridge so it connects to the new address.
    pub(crate) fn update_addr_and_reconnect_bridge(&self, agent_id: &AgentId, new_addr: String) {
        let mut agents = self.agents.lock();
        let Some(agent) = agents.get_mut(agent_id) else { return };
        agent.addr = new_addr;

        // Shut down old bridge (dropping the sender signals the bridge to exit)
        agent.bridge_shutdown.take();

        // Spawn a new bridge connected to the updated address
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(crate::adapters::agent::docker::ws::event_bridge(
            agent.addr.clone(),
            agent.auth_token.clone(),
            *agent_id,
            agent.owner,
            agent.event_tx.clone(),
            shutdown_rx,
            self.log_entry_tx.clone(),
        ));
        agent.bridge_shutdown = Some(shutdown_tx);
    }

    /// Register an agent and start its WebSocket event bridge.
    pub(crate) fn register(
        &self,
        agent_id: AgentId,
        addr: String,
        auth_token: String,
        event_tx: mpsc::Sender<Event>,
        owner: OwnerId,
    ) -> AgentHandle {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(crate::adapters::agent::docker::ws::event_bridge(
            addr.clone(),
            auth_token.clone(),
            agent_id,
            owner,
            event_tx.clone(),
            shutdown_rx,
            self.log_entry_tx.clone(),
        ));
        let handle = AgentHandle::new(agent_id);
        self.agents.lock().insert(
            agent_id,
            RemoteAgent { addr, auth_token, bridge_shutdown: Some(shutdown_tx), event_tx, owner },
        );
        handle
    }

    /// Remove an agent from the registry and send graceful shutdown to coop.
    pub(crate) async fn deregister(&self, agent_id: &AgentId) {
        let agent = self.agents.lock().remove(agent_id);

        // Request graceful coop shutdown
        if let Some(agent) = agent {
            let _ =
                http::post_authed(&agent.addr, "/api/v1/shutdown", "{}", &agent.auth_token).await;
        }
    }

    // ── The 9 shared AgentAdapter methods ──────────────────────────────

    pub(crate) async fn send(
        &self,
        agent_id: &AgentId,
        input: &str,
    ) -> Result<(), AgentAdapterError> {
        let (addr, token) = self.lookup(agent_id)?;

        let body = serde_json::json!({ "message": input }).to_string();
        let response = http::post_authed(&addr, "/api/v1/agent/nudge", &body, &token).await;

        match response {
            Ok(resp) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&resp) {
                    if json.get("delivered").and_then(|v| v.as_bool()) == Some(false) {
                        let input_body =
                            serde_json::json!({ "text": input, "enter": true }).to_string();
                        http::post_authed(&addr, "/api/v1/input", &input_body, &token).await?;
                    }
                }
                Ok(())
            }
            Err(_) => {
                let input_body = serde_json::json!({ "text": input, "enter": true }).to_string();
                http::post_authed(&addr, "/api/v1/input", &input_body, &token).await?;
                Ok(())
            }
        }
    }

    pub(crate) async fn respond(
        &self,
        agent_id: &AgentId,
        response: &oj_core::PromptResponse,
    ) -> Result<(), AgentAdapterError> {
        let (addr, token) = self.lookup(agent_id)?;

        let body = serde_json::json!({
            "accept": response.accept,
            "option": response.option,
            "text": response.text,
        })
        .to_string();

        http::post_authed(&addr, "/api/v1/agent/respond", &body, &token).await?;
        Ok(())
    }

    pub(crate) async fn get_state(
        &self,
        agent_id: &AgentId,
    ) -> Result<AgentState, AgentAdapterError> {
        let (addr, token) = self.lookup(agent_id)?;

        let body = http::get_authed(&addr, "/api/v1/agent", &token).await?;
        let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
            AgentAdapterError::SessionError(format!("invalid JSON from coop: {}", e))
        })?;

        Ok(crate::adapters::agent::parse_coop_agent_state(&json))
    }

    pub(crate) async fn last_message(&self, agent_id: &AgentId) -> Option<String> {
        let (addr, token) = self.lookup_opt(agent_id)?;

        let body = http::get_authed(&addr, "/api/v1/agent", &token).await.ok()?;
        let json: serde_json::Value = serde_json::from_str(&body).ok()?;
        let msg = json.get("last_message").and_then(|v| v.as_str()).filter(|s| !s.is_empty())?;
        Some(msg.to_string())
    }

    pub(crate) async fn resolve_stop(&self, agent_id: &AgentId) {
        let Some((addr, token)) = self.lookup_opt(agent_id) else { return };
        let _ = http::post_authed(&addr, "/api/v1/stop/resolve", "{}", &token).await;
    }

    pub(crate) async fn is_alive(&self, agent_id: &AgentId) -> bool {
        let Some((addr, token)) = self.lookup_opt(agent_id) else { return false };
        http::get_authed(&addr, "/api/v1/health", &token).await.is_ok()
    }

    pub(crate) async fn capture_output(
        &self,
        agent_id: &AgentId,
        _lines: u32,
    ) -> Result<String, AgentAdapterError> {
        let (addr, token) = self.lookup(agent_id)?;
        http::get_authed(&addr, "/api/v1/screen/text", &token).await
    }

    pub(crate) async fn fetch_transcript(
        &self,
        agent_id: &AgentId,
    ) -> Result<String, AgentAdapterError> {
        let (addr, token) = self.lookup(agent_id)?;

        let body = http::get_authed(
            &addr,
            "/api/v1/transcripts/catchup?since_transcript=0&since_line=0",
            &token,
        )
        .await?;

        parse_transcript_response(&body)
    }

    pub(crate) async fn fetch_usage(&self, agent_id: &AgentId) -> Option<UsageData> {
        let (addr, token) = self.lookup_opt(agent_id)?;

        let body = http::get_authed(&addr, "/api/v1/session/usage", &token).await.ok()?;
        let json: serde_json::Value = serde_json::from_str(&body).ok()?;
        Some(UsageData::from_json(&json))
    }
}
