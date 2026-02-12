// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Docker agent adapter — runs coop in containers with TCP transport.
//!
//! # Module layout
//!
//! - [`http`] — TCP HTTP client for containerized coop
//! - [`ws`] — TCP WebSocket event bridge
//!
//! # Architecture
//!
//! Each agent runs in a Docker container with coop listening on `--port 8080`.
//! The daemon maps a unique host port to the container's 8080 and communicates
//! via TCP HTTP/WebSocket instead of Unix sockets. A per-agent bearer token
//! (`COOP_AUTH_TOKEN`) secures the connection.
//!
//! Source code is provisioned via `git clone` into a Docker volume (init
//! container pattern), matching the K8S flow.

pub(crate) mod http;
pub(crate) mod ws;

pub use adapter::DockerAdapter;

mod adapter {
    use crate::adapters::agent::log_entry::AgentLogMessage;
    use crate::adapters::agent::remote::RemoteCoopClient;
    use crate::adapters::agent::{
        AgentAdapter, AgentAdapterError, AgentConfig, AgentHandle, AgentReconnectConfig,
    };
    use async_trait::async_trait;
    use oj_core::{AgentId, AgentState, Event};
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU16, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;

    /// Docker-specific state tracked per agent (container name, volume).
    struct DockerMeta {
        container_name: String,
        volume_name: Option<String>,
    }

    /// Agent adapter that runs coop inside Docker containers.
    ///
    /// Communicates with containerized coop over TCP instead of Unix sockets.
    /// The Docker CLI is used for container lifecycle (run, rm, volume).
    #[derive(Clone)]
    pub struct DockerAdapter {
        remote: RemoteCoopClient,
        meta: Arc<Mutex<HashMap<AgentId, DockerMeta>>>,
        port_counter: Arc<AtomicU16>,
    }

    impl Default for DockerAdapter {
        fn default() -> Self {
            Self::new()
        }
    }

    impl DockerAdapter {
        pub fn new() -> Self {
            let base_port: u16 = std::env::var("OJ_DOCKER_BASE_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(9100);
            Self {
                remote: RemoteCoopClient::new(),
                meta: Arc::new(Mutex::new(HashMap::new())),
                port_counter: Arc::new(AtomicU16::new(base_port)),
            }
        }

        pub fn with_log_entry_tx(mut self, tx: mpsc::Sender<AgentLogMessage>) -> Self {
            self.remote = self.remote.with_log_entry_tx(tx);
            self
        }

        /// Get the TCP address and auth token for a containerized agent.
        pub fn get_coop_info(&self, agent_id: &AgentId) -> Option<(String, String)> {
            self.remote.get_coop_info(agent_id)
        }

        /// Allocate the next available host port for a container.
        fn next_port(&self) -> u16 {
            self.port_counter.fetch_add(1, Ordering::Relaxed)
        }

        /// Spawn a Docker container running coop with TCP transport.
        async fn docker_spawn(
            &self,
            config: AgentConfig,
            event_tx: mpsc::Sender<Event>,
        ) -> Result<AgentHandle, AgentAdapterError> {
            let host_port = self.next_port();
            let container_port = 8080u16;
            let auth_token = crate::adapters::agent::generate_auth_token();
            let container_name = format!("oj-{}", config.agent_id);
            let addr = format!("127.0.0.1:{}", host_port);

            // Resolve the container image from the agent's container config.
            // The caller must set container config before reaching DockerAdapter.
            let image =
                std::env::var("OJ_DOCKER_IMAGE").unwrap_or_else(|_| "coop:claude".to_string());

            // Determine volume name for source provisioning
            let volume_name = format!("oj-{}-ws", config.agent_id);

            // Create Docker volume for workspace
            run_docker(&["volume", "create", &volume_name]).await.map_err(|e| {
                AgentAdapterError::SpawnFailed(format!("volume create failed: {}", e))
            })?;

            // Source provisioning: prefer repo/branch from job vars (resolved at
            // creation time), falling back to local git detection.
            let repo = if let Some(ref url) = config.repo {
                Some(url.clone())
            } else if config.workspace_path.join(".git").exists() || !config.workspace_path.exists()
            {
                crate::adapters::agent::detect_git_remote(&config.project_path).await
            } else {
                None
            };

            if let Some(repo) = repo {
                let branch = match config.branch {
                    Some(b) => Some(b),
                    None => {
                        crate::adapters::agent::detect_git_branch_async(&config.workspace_path)
                            .await
                    }
                };

                let vol_arg = format!("{}:/workspace", volume_name);
                let mut clone_args = vec!["run", "--rm", "-v", &vol_arg];

                // Mount SSH keys for clone auth
                let ssh_dir = dirs::home_dir()
                    .map(|h| h.join(".ssh"))
                    .unwrap_or_else(|| PathBuf::from("/root/.ssh"));
                let ssh_mount = format!("{}:/root/.ssh:ro", ssh_dir.display());
                if ssh_dir.exists() {
                    clone_args.extend_from_slice(&["-v", &ssh_mount]);
                }

                clone_args.push(&image);

                // Build git clone command
                let mut git_cmd =
                    format!("git clone --single-branch --depth 1 {} /workspace", repo);
                if let Some(ref branch) = branch {
                    git_cmd = format!(
                        "git clone --branch {} --single-branch --depth 1 {} /workspace",
                        branch, repo
                    );
                }

                clone_args.extend_from_slice(&["bash", "-c", &git_cmd]);

                tracing::info!(
                    agent_id = %config.agent_id,
                    %repo,
                    branch = ?branch,
                    "cloning source into Docker volume"
                );

                if let Err(e) = run_docker(&clone_args).await {
                    tracing::warn!(
                        agent_id = %config.agent_id,
                        error = %e,
                        "git clone into volume failed, continuing with empty volume"
                    );
                }
            }

            // Build coop command
            let command =
                crate::adapters::agent::augment_command_for_skip_permissions(&config.command);

            // Build docker run arguments
            let port_mapping = format!("{}:{}", host_port, container_port);
            let coop_auth_env = format!("COOP_AUTH_TOKEN={}", auth_token);
            let mut docker_args = vec![
                "run",
                "-d",
                "--name",
                &container_name,
                "-p",
                &port_mapping,
                "-e",
                &coop_auth_env,
            ];

            // Inject resolved credential from host
            let cred_env;
            if let Some(cred) = crate::adapters::credential::resolve() {
                let (key, val) = cred.to_env_pair();
                cred_env = format!("{}={}", key, val);
                docker_args.extend_from_slice(&["-e", &cred_env]);
            }

            // Forward agent environment variables
            let env_pairs: Vec<String> =
                config.env.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
            for pair in &env_pairs {
                docker_args.extend_from_slice(&["-e", pair]);
            }

            // Mount workspace volume
            let vol_mount = format!("{}:/workspace", volume_name);
            docker_args.extend_from_slice(&["-v", &vol_mount]);

            // Set working directory
            docker_args.extend_from_slice(&["-w", "/workspace"]);

            // Image and coop arguments
            docker_args.push(&image);
            let port_arg = format!("{}", container_port);
            docker_args
                .extend_from_slice(&["--port", &port_arg, "--agent", "claude", "--", "bash", "-c"]);
            let bash_cmd = format!("{} \"$@\"", command);
            docker_args.push(&bash_cmd);
            docker_args.push("_");

            tracing::info!(
                agent_id = %config.agent_id,
                %container_name,
                host_port,
                "spawning Docker container"
            );

            run_docker(&docker_args)
                .await
                .map_err(|e| AgentAdapterError::SpawnFailed(format!("docker run failed: {}", e)))?;

            // Wait for coop to be ready in the container
            let poll_ms: u64 = std::env::var("OJ_DOCKER_READY_POLL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100);
            let max_attempts: usize = std::env::var("OJ_DOCKER_READY_ATTEMPTS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(150); // 150 * 100ms = 15s
            crate::adapters::agent::poll_until_ready(
                &addr,
                &auth_token,
                poll_ms,
                max_attempts,
                "docker",
            )
            .await?;

            // Register agent and start WS bridge
            let mut handle = self.remote.register(
                config.agent_id.clone(),
                addr,
                auth_token.clone(),
                event_tx,
                config.owner,
            );
            handle.auth_token = Some(auth_token);
            self.meta.lock().insert(
                config.agent_id,
                DockerMeta { container_name, volume_name: Some(volume_name) },
            );
            Ok(handle)
        }
    }

    #[async_trait]
    impl AgentAdapter for DockerAdapter {
        async fn spawn(
            &self,
            config: AgentConfig,
            event_tx: mpsc::Sender<Event>,
        ) -> Result<AgentHandle, AgentAdapterError> {
            let span = tracing::info_span!(
                "docker.spawn",
                agent_id = %config.agent_id,
                workspace = %config.workspace_path.display()
            );
            let _guard = span.enter();
            drop(_guard);

            let start = std::time::Instant::now();
            let result = self.docker_spawn(config, event_tx).await;
            let elapsed_ms = start.elapsed().as_millis() as u64;
            match &result {
                Ok(h) => tracing::info!(agent_id = %h.agent_id, elapsed_ms, "docker agent spawned"),
                Err(e) => tracing::error!(elapsed_ms, error = %e, "docker spawn failed"),
            }
            result
        }

        async fn reconnect(
            &self,
            config: AgentReconnectConfig,
            event_tx: mpsc::Sender<Event>,
        ) -> Result<AgentHandle, AgentAdapterError> {
            // For Docker, reconnect means finding the running container's port.
            // The container name is deterministic: oj-<agent_id>
            let container_name = format!("oj-{}", config.agent_id);

            // Inspect the container to get its host port
            let output = tokio::process::Command::new("docker")
                .args(["port", &container_name, "8080"])
                .output()
                .await
                .map_err(|e| {
                    AgentAdapterError::NotFound(format!(
                        "docker port inspection failed for {}: {}",
                        container_name, e
                    ))
                })?;

            if !output.status.success() {
                return Err(AgentAdapterError::NotFound(format!(
                    "container {} not running",
                    container_name
                )));
            }

            let port_output = String::from_utf8_lossy(&output.stdout);
            // Output is like "0.0.0.0:9100" or ":::9100"
            let addr = port_output
                .lines()
                .next()
                .and_then(|line| {
                    // Handle both IPv4 and IPv6 formats
                    if line.starts_with(":::") {
                        line.strip_prefix(":::").map(|p| format!("127.0.0.1:{}", p))
                    } else {
                        Some(line.trim().to_string())
                    }
                })
                .ok_or_else(|| {
                    AgentAdapterError::NotFound(format!(
                        "could not parse port for container {}",
                        container_name
                    ))
                })?;

            let auth_token = config.auth_token.ok_or_else(|| {
                AgentAdapterError::NotFound(format!(
                    "no persisted auth token for container {}",
                    container_name
                ))
            })?;

            // Verify coop is alive
            super::http::get_authed(&addr, "/api/v1/health", &auth_token).await.map_err(|_| {
                AgentAdapterError::NotFound(format!(
                    "coop not responding in container {}",
                    container_name
                ))
            })?;

            let handle = self.remote.register(
                config.agent_id.clone(),
                addr,
                auth_token,
                event_tx,
                config.owner,
            );
            self.meta
                .lock()
                .insert(config.agent_id, DockerMeta { container_name, volume_name: None });
            Ok(handle)
        }

        async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError> {
            self.remote.send(agent_id, input).await
        }

        async fn respond(
            &self,
            agent_id: &AgentId,
            response: &oj_core::PromptResponse,
        ) -> Result<(), AgentAdapterError> {
            self.remote.respond(agent_id, response).await
        }

        async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError> {
            tracing::info!(%agent_id, "killing docker agent");
            let meta = self
                .meta
                .lock()
                .remove(agent_id)
                .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?;

            // Deregister from remote client (sends shutdown)
            self.remote.deregister(agent_id).await;

            // Wait briefly, then force remove the container
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = run_docker(&["rm", "-f", &meta.container_name]).await;

            // Clean up the workspace volume
            if let Some(ref vol) = meta.volume_name {
                let _ = run_docker(&["volume", "rm", vol]).await;
            }

            Ok(())
        }

        async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError> {
            self.remote.get_state(agent_id).await
        }

        async fn last_message(&self, agent_id: &AgentId) -> Option<String> {
            self.remote.last_message(agent_id).await
        }

        async fn resolve_stop(&self, agent_id: &AgentId) {
            self.remote.resolve_stop(agent_id).await
        }

        async fn is_alive(&self, agent_id: &AgentId) -> bool {
            self.remote.is_alive(agent_id).await
        }

        async fn capture_output(
            &self,
            agent_id: &AgentId,
            lines: u32,
        ) -> Result<String, AgentAdapterError> {
            self.remote.capture_output(agent_id, lines).await
        }

        async fn fetch_transcript(&self, agent_id: &AgentId) -> Result<String, AgentAdapterError> {
            self.remote.fetch_transcript(agent_id).await
        }

        async fn fetch_usage(
            &self,
            agent_id: &AgentId,
        ) -> Option<crate::adapters::agent::UsageData> {
            self.remote.fetch_usage(agent_id).await
        }
    }

    /// Run a docker CLI command and return stdout on success.
    async fn run_docker(args: &[&str]) -> Result<String, String> {
        let output = tokio::process::Command::new("docker")
            .args(args)
            .output()
            .await
            .map_err(|e| format!("failed to exec docker: {}", e))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("docker {} failed: {}", args.first().unwrap_or(&""), stderr.trim()))
        }
    }
}
