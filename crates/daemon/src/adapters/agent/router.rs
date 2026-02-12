// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! RuntimeRouter — delegates agent operations to the appropriate adapter
//! based on whether the agent is containerized.
//!
//! Local agents (no `container` config) go through `LocalAdapter`.
//! Containerized agents go through `DockerAdapter`.
//! The router tracks which adapter owns each agent after spawn.

use crate::adapters::agent::docker::DockerAdapter;
use crate::adapters::agent::log_entry::AgentLogMessage;
use crate::adapters::agent::{
    AgentAdapter, AgentAdapterError, AgentConfig, AgentHandle, AgentReconnectConfig,
};

/// Connection info for attaching to a coop agent.
pub struct CoopInfo {
    /// Connection URL (socket path for local, `http://addr` for remote).
    pub url: String,
    /// Auth token (empty for local agents).
    pub auth_token: String,
    /// Whether this is a remote (TCP) connection.
    pub remote: bool,
}
use async_trait::async_trait;
use oj_core::{AgentId, AgentState, Event};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

use super::coop::LocalAdapter;

/// Which adapter owns an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Local,
    Docker,
}

/// Routes agent operations to `LocalAdapter` or `DockerAdapter`.
///
/// On `spawn`, the router inspects the `container` field in `AgentConfig`.
/// Subsequent calls (send, kill, etc.) are routed based on the recorded route.
#[derive(Clone)]
pub struct RuntimeRouter {
    local: LocalAdapter,
    docker: DockerAdapter,
    routes: Arc<Mutex<HashMap<AgentId, Route>>>,
}

impl RuntimeRouter {
    pub fn new(state_dir: PathBuf) -> Self {
        Self {
            local: LocalAdapter::new(state_dir),
            docker: DockerAdapter::new(),
            routes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_log_entry_tx(mut self, tx: mpsc::Sender<AgentLogMessage>) -> Self {
        self.local = self.local.with_log_entry_tx(tx.clone());
        self.docker = self.docker.with_log_entry_tx(tx);
        self
    }

    /// Get coop connection info for an agent (local socket path or remote TCP address).
    pub fn get_coop_info(&self, agent_id: &AgentId) -> Option<CoopInfo> {
        match self.route_for(agent_id) {
            Some(Route::Docker) => {
                let (addr, token) = self.docker.get_coop_info(agent_id)?;
                Some(CoopInfo { url: format!("http://{}", addr), auth_token: token, remote: true })
            }
            Some(Route::Local) => {
                let socket_path = self.local.get_coop_socket(agent_id)?;
                Some(CoopInfo {
                    url: socket_path.to_string_lossy().to_string(),
                    auth_token: String::new(),
                    remote: false,
                })
            }
            None => None,
        }
    }

    fn route_for(&self, agent_id: &AgentId) -> Option<Route> {
        self.routes.lock().get(agent_id).copied()
    }

    fn record_route(&self, agent_id: &AgentId, route: Route) {
        self.routes.lock().insert(agent_id.clone(), route);
    }

    fn remove_route(&self, agent_id: &AgentId) {
        self.routes.lock().remove(agent_id);
    }
}

#[async_trait]
impl AgentAdapter for RuntimeRouter {
    async fn spawn(
        &self,
        config: AgentConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError> {
        let agent_id = config.agent_id.clone();
        if config.container.is_some() {
            let handle = self.docker.spawn(config, event_tx).await?;
            self.record_route(&agent_id, Route::Docker);
            Ok(handle)
        } else {
            let handle = self.local.spawn(config, event_tx).await?;
            self.record_route(&agent_id, Route::Local);
            Ok(handle)
        }
    }

    async fn reconnect(
        &self,
        config: AgentReconnectConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError> {
        let agent_id = config.agent_id.clone();

        // Try Docker first (checks for running container), fall back to local.
        match self.docker.reconnect(config.clone(), event_tx.clone()).await {
            Ok(handle) => {
                self.record_route(&agent_id, Route::Docker);
                Ok(handle)
            }
            Err(_) => {
                let handle = self.local.reconnect(config, event_tx).await?;
                self.record_route(&agent_id, Route::Local);
                Ok(handle)
            }
        }
    }

    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError> {
        match self.route_for(agent_id) {
            Some(Route::Docker) => self.docker.send(agent_id, input).await,
            Some(Route::Local) => self.local.send(agent_id, input).await,
            None => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        }
    }

    async fn respond(
        &self,
        agent_id: &AgentId,
        response: &oj_core::PromptResponse,
    ) -> Result<(), AgentAdapterError> {
        match self.route_for(agent_id) {
            Some(Route::Docker) => self.docker.respond(agent_id, response).await,
            Some(Route::Local) => self.local.respond(agent_id, response).await,
            None => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        }
    }

    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError> {
        let route = self.route_for(agent_id);
        let result = match route {
            Some(Route::Docker) => self.docker.kill(agent_id).await,
            Some(Route::Local) => self.local.kill(agent_id).await,
            None => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        };
        self.remove_route(agent_id);
        result
    }

    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError> {
        match self.route_for(agent_id) {
            Some(Route::Docker) => self.docker.get_state(agent_id).await,
            Some(Route::Local) => self.local.get_state(agent_id).await,
            None => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        }
    }

    async fn last_message(&self, agent_id: &AgentId) -> Option<String> {
        match self.route_for(agent_id) {
            Some(Route::Docker) => self.docker.last_message(agent_id).await,
            Some(Route::Local) => self.local.last_message(agent_id).await,
            None => None,
        }
    }

    async fn resolve_stop(&self, agent_id: &AgentId) {
        match self.route_for(agent_id) {
            Some(Route::Docker) => self.docker.resolve_stop(agent_id).await,
            Some(Route::Local) => self.local.resolve_stop(agent_id).await,
            None => {}
        }
    }

    async fn is_alive(&self, agent_id: &AgentId) -> bool {
        match self.route_for(agent_id) {
            Some(Route::Docker) => self.docker.is_alive(agent_id).await,
            Some(Route::Local) => self.local.is_alive(agent_id).await,
            None => false,
        }
    }

    async fn capture_output(
        &self,
        agent_id: &AgentId,
        lines: u32,
    ) -> Result<String, AgentAdapterError> {
        match self.route_for(agent_id) {
            Some(Route::Docker) => self.docker.capture_output(agent_id, lines).await,
            Some(Route::Local) => self.local.capture_output(agent_id, lines).await,
            None => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        }
    }

    async fn fetch_transcript(&self, agent_id: &AgentId) -> Result<String, AgentAdapterError> {
        match self.route_for(agent_id) {
            Some(Route::Docker) => self.docker.fetch_transcript(agent_id).await,
            Some(Route::Local) => self.local.fetch_transcript(agent_id).await,
            None => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        }
    }

    async fn fetch_usage(&self, agent_id: &AgentId) -> Option<crate::adapters::agent::UsageData> {
        match self.route_for(agent_id) {
            Some(Route::Docker) => self.docker.fetch_usage(agent_id).await,
            Some(Route::Local) => self.local.fetch_usage(agent_id).await,
            None => None,
        }
    }
}
