// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Kubernetes agent adapter — runs coop in pods with TCP transport.
//!
//! # Module layout
//!
//! - [`pod`] — Pod spec construction helpers
//!
//! # Architecture
//!
//! Each agent runs in a Kubernetes pod with coop listening on `--port 8080`.
//! The daemon creates pods via the Kubernetes API and communicates with each
//! coop via TCP HTTP/WebSocket on the pod's cluster IP. Per-agent bearer
//! tokens (`COOP_AUTH_TOKEN`) secure the connections. Credentials are injected
//! from Kubernetes Secrets rather than resolved from the host.

mod pod;

pub use adapter::KubernetesAdapter;

mod adapter {
    use super::pod::{self, CrewEnv, PodParams};
    use crate::adapters::agent::docker::http;
    use crate::adapters::agent::log_entry::AgentLogMessage;
    use crate::adapters::agent::remote::RemoteCoopClient;
    use crate::adapters::agent::{
        AgentAdapter, AgentAdapterError, AgentConfig, AgentHandle, AgentReconnectConfig,
    };
    use async_trait::async_trait;
    use k8s_openapi::api::core::v1::Pod;
    use kube::api::{Api, PostParams};
    use kube::Client;
    use oj_core::{AgentId, AgentState, Event};
    use parking_lot::Mutex;
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;

    /// Kubernetes-specific state tracked per agent (pod name, namespace).
    #[derive(Clone)]
    struct KubeMeta {
        pod_name: String,
        k8s_namespace: String,
    }

    /// Agent adapter that runs coop inside Kubernetes pods.
    ///
    /// Communicates with containerized coop over TCP (same as DockerAdapter).
    /// Uses `kube-rs` for pod lifecycle management via the Kubernetes API.
    #[derive(Clone)]
    pub struct KubernetesAdapter {
        remote: RemoteCoopClient,
        meta: Arc<Mutex<HashMap<AgentId, KubeMeta>>>,
        client: Client,
    }

    impl KubernetesAdapter {
        pub async fn new() -> Result<Self, AgentAdapterError> {
            let client = Client::try_default().await.map_err(|e| {
                AgentAdapterError::SpawnFailed(format!("failed to create kube client: {}", e))
            })?;
            Ok(Self {
                remote: RemoteCoopClient::new(),
                meta: Arc::new(Mutex::new(HashMap::new())),
                client,
            })
        }

        pub fn with_log_entry_tx(mut self, tx: mpsc::Sender<AgentLogMessage>) -> Self {
            self.remote = self.remote.with_log_entry_tx(tx);
            self
        }

        /// Get the TCP address and auth token for a Kubernetes agent.
        pub fn get_coop_host(&self, agent_id: &AgentId) -> Option<(String, String)> {
            self.remote.get_coop_host(agent_id)
        }

        /// Kubernetes project for agent pods.
        fn k8s_namespace() -> String {
            std::env::var("OJ_K8S_NAMESPACE").unwrap_or_else(|_| "default".to_string())
        }

        /// Container image for agent pods.
        fn image() -> String {
            std::env::var("OJ_K8S_IMAGE").unwrap_or_else(|_| "coop:claude".to_string())
        }

        /// Credential secret name for Kubernetes.
        fn credential_secret() -> Option<String> {
            std::env::var("OJ_K8S_CREDENTIAL_SECRET").ok()
        }

        /// SSH deploy key secret name.
        fn ssh_secret() -> Option<String> {
            std::env::var("OJ_K8S_SSH_SECRET").ok()
        }

        /// Daemon URL for crew agents.
        fn daemon_url() -> Option<String> {
            std::env::var("OJ_DAEMON_URL").ok()
        }

        /// Auth token for crew agents to connect to the daemon.
        fn daemon_auth_token() -> Option<String> {
            std::env::var("OJ_AUTH_TOKEN").ok()
        }

        /// Container port for coop inside the pod.
        fn container_port() -> i32 {
            std::env::var("OJ_K8S_COOP_PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8080)
        }

        /// Look up the current pod IP via the K8s API and update the agent's
        /// registered address. Returns `true` if the address was refreshed.
        ///
        /// Called on connection failure to handle pod rescheduling (where the
        /// pod IP changes but the pod name stays the same).
        async fn refresh_pod_ip(&self, agent_id: &AgentId) -> bool {
            let meta = self.meta.lock().get(agent_id).cloned();
            let Some(meta) = meta else { return false };

            let pods: Api<Pod> = Api::namespaced(self.client.clone(), &meta.k8s_namespace);
            let pod = match pods.get(&meta.pod_name).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::debug!(%agent_id, error = %e, "pod lookup failed during IP refresh");
                    return false;
                }
            };

            let ip = pod.status.as_ref().and_then(|s| s.pod_ip.as_ref());
            if let Some(ip) = ip {
                let port = Self::container_port();
                let new_addr = format!("{}:{}", ip, port);
                tracing::info!(%agent_id, %new_addr, "refreshed pod IP");
                self.remote.update_addr_and_reconnect_bridge(agent_id, new_addr);
                true
            } else {
                false
            }
        }

        /// Create a pod and wait for it to become ready.
        async fn k8s_spawn(
            &self,
            config: AgentConfig,
            event_tx: mpsc::Sender<Event>,
        ) -> Result<AgentHandle, AgentAdapterError> {
            let auth_token = crate::adapters::agent::generate_auth_token();
            let k8s_namespace = Self::k8s_namespace();
            let container_port = Self::container_port();
            let pod_name = format!("oj-{}", config.agent_id);
            let image = Self::image();

            // Build git clone command for init container.
            // Prefer repo/branch from job vars (resolved at creation time),
            // falling back to local git detection for backwards compatibility.
            let git_clone_cmd = {
                let repo = if let Some(ref url) = config.repo {
                    Some(url.clone())
                } else if config.workspace_path.join(".git").exists()
                    || !config.workspace_path.exists()
                {
                    crate::adapters::agent::detect_git_remote(&config.project_path).await
                } else {
                    None
                };
                repo.map(|url| {
                    let branch = config.branch.clone().or_else(|| {
                        crate::adapters::agent::detect_git_branch_blocking(&config.workspace_path)
                    });
                    pod::git_clone_command(&url, branch.as_deref())
                })
            };

            // Determine if this is a crew agent (needs daemon URL)
            let crew_env = Self::daemon_url().and_then(|url| {
                Self::daemon_auth_token()
                    .map(|token| CrewEnv { daemon_url: url, auth_token: token })
            });

            // Build agent command
            let command =
                crate::adapters::agent::augment_command_for_skip_permissions(&config.command);

            let params = PodParams {
                pod_name: pod_name.clone(),
                image,
                namespace: k8s_namespace.clone(),
                agent_command: command,
                auth_token: auth_token.clone(),
                container_port,
                credential_secret: Self::credential_secret(),
                ssh_secret: Self::ssh_secret(),
                git_clone_cmd,
                env: config.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                crew_env,
                project: config
                    .env
                    .iter()
                    .find(|(k, _)| k == "OJ_PROJECT")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default(),
            };

            let pod_spec = pod::build_pod(&params);

            // Create the pod via Kubernetes API
            let pods: Api<Pod> = Api::namespaced(self.client.clone(), &k8s_namespace);
            let pp = PostParams::default();

            tracing::info!(
                agent_id = %config.agent_id,
                %pod_name,
                k8s_namespace = %k8s_namespace,
                "creating Kubernetes pod"
            );

            pods.create(&pp, &pod_spec).await.map_err(|e| {
                AgentAdapterError::SpawnFailed(format!("pod creation failed: {}", e))
            })?;

            // After pod creation succeeds, any failure must clean up the pod.
            let result = async {
                // Wait for pod to get an IP
                let pod_ip = wait_for_pod_ip(&pods, &pod_name).await?;
                let addr = format!("{}:{}", pod_ip, container_port);

                // Wait for coop to be ready
                let poll_ms: u64 = std::env::var("OJ_K8S_READY_POLL_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(500);
                let max_attempts: usize = std::env::var("OJ_K8S_COOP_READY_ATTEMPTS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(120); // 120 * 500ms = 60s
                crate::adapters::agent::poll_until_ready(
                    &addr,
                    &auth_token,
                    poll_ms,
                    max_attempts,
                    "k8s",
                )
                .await?;

                // Register agent and start WS bridge
                let mut handle = self.remote.register(
                    config.agent_id,
                    addr,
                    auth_token.clone(),
                    event_tx,
                    config.owner,
                );
                handle.auth_token = Some(auth_token);
                self.meta.lock().insert(
                    config.agent_id,
                    KubeMeta { pod_name: pod_name.clone(), k8s_namespace },
                );
                Ok(handle)
            }
            .await;

            if result.is_err() {
                let dp = kube::api::DeleteParams::default();
                if let Err(del_err) = pods.delete(&pod_name, &dp).await {
                    tracing::warn!(
                        %pod_name,
                        error = %del_err,
                        "failed to clean up pod after spawn failure"
                    );
                }
            }
            result
        }
    }

    #[async_trait]
    impl AgentAdapter for KubernetesAdapter {
        async fn spawn(
            &self,
            config: AgentConfig,
            event_tx: mpsc::Sender<Event>,
        ) -> Result<AgentHandle, AgentAdapterError> {
            let start = std::time::Instant::now();
            let result = self.k8s_spawn(config, event_tx).await;
            let elapsed_ms = start.elapsed().as_millis() as u64;
            match &result {
                Ok(h) => tracing::info!(agent_id = %h.agent_id, elapsed_ms, "k8s agent spawned"),
                Err(e) => tracing::error!(elapsed_ms, error = %e, "k8s spawn failed"),
            }
            result
        }

        async fn cleanup_stale_resources(&self, known_agents: &HashSet<AgentId>) {
            let k8s_namespace = Self::k8s_namespace();
            let pods: Api<Pod> = Api::namespaced(self.client.clone(), &k8s_namespace);
            let lp = kube::api::ListParams::default().labels("app=oj-agent");
            let pod_list = match pods.list(&lp).await {
                Ok(list) => list,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to list pods for stale resource cleanup");
                    return;
                }
            };

            for pod in pod_list {
                let pod_name = match pod.metadata.name {
                    Some(ref name) => name.clone(),
                    None => continue,
                };
                // Pod names are "oj-<agent_id>" — extract agent ID
                let agent_id_str = match pod_name.strip_prefix("oj-") {
                    Some(id) => id,
                    None => continue,
                };
                let agent_id = AgentId::from_string(agent_id_str);
                if !known_agents.contains(&agent_id) {
                    tracing::info!(%pod_name, "deleting orphaned pod");
                    let dp = kube::api::DeleteParams::default();
                    if let Err(e) = pods.delete(&pod_name, &dp).await {
                        tracing::warn!(%pod_name, error = %e, "failed to delete orphaned pod");
                    }
                }
            }
        }

        async fn reconnect(
            &self,
            config: AgentReconnectConfig,
            event_tx: mpsc::Sender<Event>,
        ) -> Result<AgentHandle, AgentAdapterError> {
            let k8s_namespace = Self::k8s_namespace();
            let container_port = Self::container_port();
            let pod_name = format!("oj-{}", config.agent_id);

            let pods: Api<Pod> = Api::namespaced(self.client.clone(), &k8s_namespace);

            // Get the pod and its IP
            let existing = pods.get(&pod_name).await.map_err(|e| {
                AgentAdapterError::NotFound(format!("pod {} not found: {}", pod_name, e))
            })?;

            let pod_ip =
                existing.status.as_ref().and_then(|s| s.pod_ip.as_ref()).ok_or_else(|| {
                    AgentAdapterError::NotFound(format!("pod {} has no IP", pod_name))
                })?;

            let addr = format!("{}:{}", pod_ip, container_port);

            let auth_token = config.auth_token.ok_or_else(|| {
                AgentAdapterError::NotFound(format!("no persisted auth token for pod {}", pod_name))
            })?;

            // Verify coop is alive
            http::get_authed(&addr, "/api/v1/health", &auth_token).await.map_err(|_| {
                AgentAdapterError::NotFound(format!("coop not responding in pod {}", pod_name))
            })?;

            let handle =
                self.remote.register(config.agent_id, addr, auth_token, event_tx, config.owner);
            self.meta.lock().insert(config.agent_id, KubeMeta { pod_name, k8s_namespace });
            Ok(handle)
        }

        async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError> {
            match self.remote.send(agent_id, input).await {
                Ok(v) => Ok(v),
                Err(e) if self.refresh_pod_ip(agent_id).await => {
                    self.remote.send(agent_id, input).await
                }
                Err(e) => Err(e),
            }
        }

        async fn respond(
            &self,
            agent_id: &AgentId,
            response: &oj_core::PromptResponse,
        ) -> Result<(), AgentAdapterError> {
            match self.remote.respond(agent_id, response).await {
                Ok(v) => Ok(v),
                Err(e) if self.refresh_pod_ip(agent_id).await => {
                    self.remote.respond(agent_id, response).await
                }
                Err(e) => Err(e),
            }
        }

        async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError> {
            tracing::info!(%agent_id, "killing k8s agent");
            let meta = self
                .meta
                .lock()
                .remove(agent_id)
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?;

            // Deregister from remote client (sends shutdown)
            self.remote.deregister(agent_id).await;

            // Delete the pod
            let pods: Api<Pod> = Api::namespaced(self.client.clone(), &meta.k8s_namespace);
            let dp = kube::api::DeleteParams::default();
            if let Err(e) = pods.delete(&meta.pod_name, &dp).await {
                tracing::warn!(%agent_id, pod = %meta.pod_name, error = %e, "failed to delete pod");
            }

            Ok(())
        }

        async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError> {
            match self.remote.get_state(agent_id).await {
                Ok(v) => Ok(v),
                Err(e) if self.refresh_pod_ip(agent_id).await => {
                    self.remote.get_state(agent_id).await
                }
                Err(e) => Err(e),
            }
        }

        async fn last_message(&self, agent_id: &AgentId) -> Option<String> {
            // No retry: None is ambiguous (no messages yet vs network error).
            // Pod IP refresh is handled by Result-returning methods and is_alive.
            self.remote.last_message(agent_id).await
        }

        async fn resolve_stop(&self, agent_id: &AgentId) {
            self.remote.resolve_stop(agent_id).await
        }

        async fn is_alive(&self, agent_id: &AgentId) -> bool {
            if self.remote.is_alive(agent_id).await {
                return true;
            }
            // Pod may have been rescheduled — refresh IP and retry
            if self.refresh_pod_ip(agent_id).await {
                return self.remote.is_alive(agent_id).await;
            }
            false
        }

        async fn capture_output(
            &self,
            agent_id: &AgentId,
            lines: u32,
        ) -> Result<String, AgentAdapterError> {
            match self.remote.capture_output(agent_id, lines).await {
                Ok(v) => Ok(v),
                Err(e) if self.refresh_pod_ip(agent_id).await => {
                    self.remote.capture_output(agent_id, lines).await
                }
                Err(e) => Err(e),
            }
        }

        async fn fetch_transcript(&self, agent_id: &AgentId) -> Result<String, AgentAdapterError> {
            match self.remote.fetch_transcript(agent_id).await {
                Ok(v) => Ok(v),
                Err(e) if self.refresh_pod_ip(agent_id).await => {
                    self.remote.fetch_transcript(agent_id).await
                }
                Err(e) => Err(e),
            }
        }

        async fn fetch_usage(
            &self,
            agent_id: &AgentId,
        ) -> Option<crate::adapters::agent::UsageData> {
            // No retry: None is ambiguous (no usage data vs network error).
            self.remote.fetch_usage(agent_id).await
        }
    }

    /// Wait for a pod to receive an IP address.
    async fn wait_for_pod_ip(pods: &Api<Pod>, name: &str) -> Result<String, AgentAdapterError> {
        let poll_ms: u64 =
            std::env::var("OJ_K8S_READY_POLL_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(500);
        let max_attempts: usize =
            std::env::var("OJ_K8S_READY_ATTEMPTS").ok().and_then(|v| v.parse().ok()).unwrap_or(120); // 120 * 500ms = 60s

        for i in 0..max_attempts {
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(poll_ms)).await;
            }
            if let Ok(pod) = pods.get(name).await {
                if let Some(ip) = pod.status.as_ref().and_then(|s| s.pod_ip.as_ref()) {
                    if !ip.is_empty() {
                        tracing::info!(%name, %ip, attempt = i, "pod IP assigned");
                        return Ok(ip.clone());
                    }
                }
            }
        }
        Err(AgentAdapterError::SpawnFailed(format!(
            "pod {} did not receive IP within {}s",
            name,
            (max_attempts as u64 * poll_ms) / 1000
        )))
    }
}
