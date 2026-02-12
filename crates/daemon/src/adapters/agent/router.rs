// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! RuntimeRouter — delegates agent operations to the appropriate adapter.
//!
//! When running inside a Kubernetes cluster (`with_k8s` succeeds), all agents
//! route through `KubernetesAdapter` — there is no "local" inside a pod.
//!
//! Otherwise, local agents (no `container` config) go through `LocalAdapter`
//! and containerized agents go through `DockerAdapter`.
//!
//! The router tracks which adapter owns each agent after spawn.

use crate::adapters::agent::docker::DockerAdapter;
use crate::adapters::agent::k8s::KubernetesAdapter;
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
    Kubernetes,
}

/// Routes agent operations to the appropriate adapter.
///
/// On `spawn`, the router selects an adapter: K8s (when running in-cluster),
/// Docker (when `container` config is present), or Local (default).
/// Subsequent calls (send, kill, etc.) are routed based on the recorded route.
#[derive(Clone)]
pub struct RuntimeRouter {
    local: LocalAdapter,
    docker: DockerAdapter,
    k8s: Option<KubernetesAdapter>,
    routes: Arc<Mutex<HashMap<AgentId, Route>>>,
}

impl RuntimeRouter {
    pub fn new(state_dir: PathBuf) -> Self {
        Self {
            local: LocalAdapter::new(state_dir),
            docker: DockerAdapter::new(),
            k8s: None,
            routes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Initialize the Kubernetes adapter if running inside a cluster.
    ///
    /// Only activates when `KUBERNETES_SERVICE_HOST` is set (injected by the
    /// kubelet into every pod). When successful, all agents route through
    /// K8s — there is no "local" inside a pod. No-op on developer machines.
    ///
    /// Returns an error if in-cluster is detected but the adapter fails to
    /// initialize — the daemon cannot function without K8s routing inside a pod.
    pub async fn with_k8s(mut self) -> Result<Self, AgentAdapterError> {
        // Only enable K8s routing when running inside a cluster.
        if std::env::var("KUBERNETES_SERVICE_HOST").is_err() {
            return Ok(self);
        }

        let adapter = KubernetesAdapter::new().await?;
        tracing::info!("Kubernetes adapter initialized (in-cluster)");
        self.k8s = Some(adapter);
        Ok(self)
    }

    pub fn with_log_entry_tx(mut self, tx: mpsc::Sender<AgentLogMessage>) -> Self {
        self.local = self.local.with_log_entry_tx(tx.clone());
        self.docker = self.docker.with_log_entry_tx(tx.clone());
        if let Some(k8s) = self.k8s.take() {
            self.k8s = Some(k8s.with_log_entry_tx(tx));
        }
        self
    }

    /// Get coop connection info for an agent (local socket path or remote TCP address).
    pub fn get_coop_info(&self, agent_id: &AgentId) -> Option<CoopInfo> {
        match self.route_for(agent_id) {
            Some(Route::Kubernetes) => {
                let (addr, token) = self.k8s.as_ref()?.get_coop_info(agent_id)?;
                Some(CoopInfo { url: format!("http://{}", addr), auth_token: token, remote: true })
            }
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
        // When running in-cluster, all agents go through K8s.
        if let Some(k8s) = &self.k8s {
            let mut handle = k8s.spawn(config, event_tx).await?;
            handle.runtime = oj_core::AgentRuntime::Kubernetes;
            self.record_route(&agent_id, Route::Kubernetes);
            Ok(handle)
        } else if config.container.is_some() {
            let mut handle = self.docker.spawn(config, event_tx).await?;
            handle.runtime = oj_core::AgentRuntime::Docker;
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
        // When running in-cluster, only K8s agents exist.
        if let Some(k8s) = &self.k8s {
            let mut handle = k8s.reconnect(config, event_tx).await?;
            handle.runtime = oj_core::AgentRuntime::Kubernetes;
            self.record_route(&agent_id, Route::Kubernetes);
            Ok(handle)
        } else {
            // Use runtime_hint to try the correct adapter first, with fallback.
            let try_docker_first = matches!(config.runtime_hint, oj_core::AgentRuntime::Docker);
            if try_docker_first {
                // Docker hint: try Docker first, fall back to Local
                match self.docker.reconnect(config.clone(), event_tx.clone()).await {
                    Ok(mut handle) => {
                        handle.runtime = oj_core::AgentRuntime::Docker;
                        self.record_route(&agent_id, Route::Docker);
                        Ok(handle)
                    }
                    Err(_) => {
                        let handle = self.local.reconnect(config, event_tx).await?;
                        self.record_route(&agent_id, Route::Local);
                        Ok(handle)
                    }
                }
            } else {
                // Local (or unknown) hint: try Local first, fall back to Docker
                match self.local.reconnect(config.clone(), event_tx.clone()).await {
                    Ok(handle) => {
                        self.record_route(&agent_id, Route::Local);
                        Ok(handle)
                    }
                    Err(_) => {
                        let mut handle = self.docker.reconnect(config, event_tx).await?;
                        handle.runtime = oj_core::AgentRuntime::Docker;
                        self.record_route(&agent_id, Route::Docker);
                        Ok(handle)
                    }
                }
            }
        }
    }

    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError> {
        match (self.route_for(agent_id), &self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.send(agent_id, input).await,
            (Some(Route::Docker), _) => self.docker.send(agent_id, input).await,
            (Some(Route::Local), _) => self.local.send(agent_id, input).await,
            _ => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        }
    }

    async fn respond(
        &self,
        agent_id: &AgentId,
        response: &oj_core::PromptResponse,
    ) -> Result<(), AgentAdapterError> {
        match (self.route_for(agent_id), &self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.respond(agent_id, response).await,
            (Some(Route::Docker), _) => self.docker.respond(agent_id, response).await,
            (Some(Route::Local), _) => self.local.respond(agent_id, response).await,
            _ => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        }
    }

    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError> {
        let result = match (self.route_for(agent_id), &self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.kill(agent_id).await,
            (Some(Route::Docker), _) => self.docker.kill(agent_id).await,
            (Some(Route::Local), _) => self.local.kill(agent_id).await,
            _ => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        };
        self.remove_route(agent_id);
        result
    }

    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError> {
        match (self.route_for(agent_id), &self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.get_state(agent_id).await,
            (Some(Route::Docker), _) => self.docker.get_state(agent_id).await,
            (Some(Route::Local), _) => self.local.get_state(agent_id).await,
            _ => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        }
    }

    async fn last_message(&self, agent_id: &AgentId) -> Option<String> {
        match (self.route_for(agent_id), &self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.last_message(agent_id).await,
            (Some(Route::Docker), _) => self.docker.last_message(agent_id).await,
            (Some(Route::Local), _) => self.local.last_message(agent_id).await,
            _ => None,
        }
    }

    async fn resolve_stop(&self, agent_id: &AgentId) {
        match (self.route_for(agent_id), &self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.resolve_stop(agent_id).await,
            (Some(Route::Docker), _) => self.docker.resolve_stop(agent_id).await,
            (Some(Route::Local), _) => self.local.resolve_stop(agent_id).await,
            _ => {}
        }
    }

    async fn is_alive(&self, agent_id: &AgentId) -> bool {
        match (self.route_for(agent_id), &self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.is_alive(agent_id).await,
            (Some(Route::Docker), _) => self.docker.is_alive(agent_id).await,
            (Some(Route::Local), _) => self.local.is_alive(agent_id).await,
            _ => false,
        }
    }

    async fn capture_output(
        &self,
        agent_id: &AgentId,
        lines: u32,
    ) -> Result<String, AgentAdapterError> {
        match (self.route_for(agent_id), &self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.capture_output(agent_id, lines).await,
            (Some(Route::Docker), _) => self.docker.capture_output(agent_id, lines).await,
            (Some(Route::Local), _) => self.local.capture_output(agent_id, lines).await,
            _ => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        }
    }

    async fn fetch_transcript(&self, agent_id: &AgentId) -> Result<String, AgentAdapterError> {
        match (self.route_for(agent_id), &self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.fetch_transcript(agent_id).await,
            (Some(Route::Docker), _) => self.docker.fetch_transcript(agent_id).await,
            (Some(Route::Local), _) => self.local.fetch_transcript(agent_id).await,
            _ => Err(AgentAdapterError::NotFound(agent_id.to_string())),
        }
    }

    async fn fetch_usage(&self, agent_id: &AgentId) -> Option<crate::adapters::agent::UsageData> {
        match (self.route_for(agent_id), &self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.fetch_usage(agent_id).await,
            (Some(Route::Docker), _) => self.docker.fetch_usage(agent_id).await,
            (Some(Route::Local), _) => self.local.fetch_usage(agent_id).await,
            _ => None,
        }
    }
}
