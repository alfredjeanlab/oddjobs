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
use std::collections::{HashMap, HashSet};
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

    fn route_for(&self, agent_id: &AgentId) -> Option<Route> {
        self.routes.lock().get(agent_id).copied()
    }

    fn record_route(&self, agent_id: &AgentId, route: Route) {
        self.routes.lock().insert(*agent_id, route);
    }

    fn remove_route(&self, agent_id: &AgentId) {
        self.routes.lock().remove(agent_id);
    }
}

/// Dispatch a method call to the adapter that owns `agent_id`.
///
/// Eliminates the repeated `match (route, &self.k8s)` blocks by looking up
/// the route once and delegating to the matching adapter.
///
/// Variants:
///   `dispatch!(self, agent_id, method(args..))` — returns `Result<T, AgentAdapterError>`
///   `dispatch!(self, agent_id, method(args..) or $fallback)` — returns the fallback on miss
macro_rules! dispatch {
    // Result-returning: missing route → NotFound
    ($self:ident, $id:expr, $method:ident( $($arg:expr),* $(,)? )) => {
        match ($self.route_for($id), &$self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.$method($($arg),*).await,
            (Some(Route::Docker), _) => $self.docker.$method($($arg),*).await,
            (Some(Route::Local), _) => $self.local.$method($($arg),*).await,
            _ => Err(AgentAdapterError::NotFound($id.to_string())),
        }
    };
    // Fallback-returning: missing route → literal
    ($self:ident, $id:expr, $method:ident( $($arg:expr),* $(,)? ) or $fallback:expr) => {
        match ($self.route_for($id), &$self.k8s) {
            (Some(Route::Kubernetes), Some(k8s)) => k8s.$method($($arg),*).await,
            (Some(Route::Docker), _) => $self.docker.$method($($arg),*).await,
            (Some(Route::Local), _) => $self.local.$method($($arg),*).await,
            _ => $fallback,
        }
    };
}

#[async_trait]
impl AgentAdapter for RuntimeRouter {
    async fn spawn(
        &self,
        config: AgentConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError> {
        let agent_id = config.agent_id;
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

    async fn cleanup_stale_resources(&self, known_agents: &HashSet<AgentId>) {
        if let Some(k8s) = &self.k8s {
            k8s.cleanup_stale_resources(known_agents).await;
        }
    }

    async fn reconnect(
        &self,
        config: AgentReconnectConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError> {
        let agent_id = config.agent_id;
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
        dispatch!(self, agent_id, send(agent_id, input))
    }

    async fn respond(
        &self,
        agent_id: &AgentId,
        response: &oj_core::PromptResponse,
    ) -> Result<(), AgentAdapterError> {
        dispatch!(self, agent_id, respond(agent_id, response))
    }

    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError> {
        let result = dispatch!(self, agent_id, kill(agent_id));
        self.remove_route(agent_id);
        result
    }

    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError> {
        dispatch!(self, agent_id, get_state(agent_id))
    }

    async fn last_message(&self, agent_id: &AgentId) -> Option<String> {
        dispatch!(self, agent_id, last_message(agent_id) or None)
    }

    async fn resolve_stop(&self, agent_id: &AgentId) {
        dispatch!(self, agent_id, resolve_stop(agent_id) or ());
    }

    async fn is_alive(&self, agent_id: &AgentId) -> bool {
        dispatch!(self, agent_id, is_alive(agent_id) or false)
    }

    async fn capture_output(
        &self,
        agent_id: &AgentId,
        lines: u32,
    ) -> Result<String, AgentAdapterError> {
        dispatch!(self, agent_id, capture_output(agent_id, lines))
    }

    async fn fetch_transcript(&self, agent_id: &AgentId) -> Result<String, AgentAdapterError> {
        dispatch!(self, agent_id, fetch_transcript(agent_id))
    }

    async fn fetch_usage(&self, agent_id: &AgentId) -> Option<crate::adapters::agent::UsageData> {
        dispatch!(self, agent_id, fetch_usage(agent_id) or None)
    }

    /// Get coop connection info for an agent (local socket path or remote TCP address).
    fn get_coop_host(&self, agent_id: &AgentId) -> Option<CoopInfo> {
        match self.route_for(agent_id) {
            Some(Route::Kubernetes) => {
                let (addr, token) = self.k8s.as_ref()?.get_coop_host(agent_id)?;
                Some(CoopInfo { url: format!("http://{}", addr), auth_token: token, remote: true })
            }
            Some(Route::Docker) => {
                let (addr, token) = self.docker.get_coop_host(agent_id)?;
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

    /// Whether the daemon is running in remote-only mode (e.g. inside a k8s pod).
    ///
    /// When true, agent pods provision their own code — the daemon should skip
    /// local filesystem workspace operations (worktree creation, deletion).
    fn is_remote_only(&self) -> bool {
        self.k8s.is_some()
    }
}
