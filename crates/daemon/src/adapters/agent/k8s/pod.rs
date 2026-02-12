// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Pod spec construction for Kubernetes agents.

use k8s_openapi::api::core::v1::{
    Container, ContainerPort, EmptyDirVolumeSource, EnvVar, EnvVarSource, HTTPGetAction, Pod,
    PodSpec, Probe, SecretKeySelector, SecretVolumeSource, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

/// Parameters for building a Kubernetes agent pod.
pub(super) struct PodParams {
    pub pod_name: String,
    pub image: String,
    pub namespace: String,
    pub agent_command: String,
    pub auth_token: String,
    pub container_port: i32,
    /// Credential secret name (e.g. "oj-credentials")
    pub credential_secret: Option<String>,
    /// SSH deploy key secret name (e.g. "oj-deploy-key")
    pub ssh_secret: Option<String>,
    /// Git clone command for init container (None = no source provisioning)
    pub git_clone_cmd: Option<Vec<String>>,
    /// Extra environment variables from agent config
    pub env: Vec<(String, String)>,
    /// Whether this is a crew agent that needs OJ_DAEMON_URL etc.
    pub crew_env: Option<CrewEnv>,
    /// Project scoping for OJ_PROJECT
    pub project: String,
}

/// Environment for crew agents that call back to the daemon.
pub(super) struct CrewEnv {
    pub daemon_url: String,
    pub auth_token: String,
}

/// Build a Pod spec for a Kubernetes agent.
pub(super) fn build_pod(params: &PodParams) -> Pod {
    let mut volumes = Vec::new();
    let mut init_containers = Vec::new();

    // Workspace volume (always present for source provisioning)
    volumes.push(Volume {
        name: "workspace".to_string(),
        empty_dir: Some(EmptyDirVolumeSource::default()),
        ..Default::default()
    });

    // SSH key volume (if configured)
    if let Some(ref ssh_secret) = params.ssh_secret {
        volumes.push(Volume {
            name: "ssh-key".to_string(),
            secret: Some(SecretVolumeSource {
                secret_name: Some(ssh_secret.clone()),
                default_mode: Some(0o400),
                ..Default::default()
            }),
            ..Default::default()
        });
    }

    // Init container for git clone
    if let Some(ref clone_cmd) = params.git_clone_cmd {
        let mut init_volume_mounts = vec![VolumeMount {
            name: "workspace".to_string(),
            mount_path: "/workspace".to_string(),
            ..Default::default()
        }];

        if params.ssh_secret.is_some() {
            init_volume_mounts.push(VolumeMount {
                name: "ssh-key".to_string(),
                mount_path: "/root/.ssh".to_string(),
                read_only: Some(true),
                ..Default::default()
            });
        }

        init_containers.push(Container {
            name: "clone".to_string(),
            image: Some(params.image.clone()),
            command: Some(clone_cmd.clone()),
            volume_mounts: Some(init_volume_mounts),
            ..Default::default()
        });
    }

    // Main agent container environment
    let mut env = vec![EnvVar {
        name: "COOP_AUTH_TOKEN".to_string(),
        value: Some(params.auth_token.clone()),
        ..Default::default()
    }];

    // Credential injection from Kubernetes Secret
    if let Some(ref secret_name) = params.credential_secret {
        // Try OAuth token first, then API key
        env.push(EnvVar {
            name: "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: secret_name.clone(),
                    key: "oauth-token".to_string(),
                    optional: Some(true),
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
        env.push(EnvVar {
            name: "ANTHROPIC_API_KEY".to_string(),
            value_from: Some(EnvVarSource {
                secret_key_ref: Some(SecretKeySelector {
                    name: secret_name.clone(),
                    key: "api-key".to_string(),
                    optional: Some(true),
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
    }

    // Agent environment (for agents that call back to daemon)
    if let Some(ref crew) = params.crew_env {
        env.push(env_var("OJ_DAEMON_URL", &crew.daemon_url));
        env.push(env_var("OJ_AUTH_TOKEN", &crew.auth_token));
        env.push(env_var("OJ_PROJECT", &params.project));
    }

    // Forward extra agent environment variables
    for (k, v) in &params.env {
        env.push(env_var(k, v));
    }

    // Build coop command line
    let port_str = params.container_port.to_string();
    let args = vec![
        "--port".to_string(),
        port_str,
        "--agent".to_string(),
        "claude".to_string(),
        "--".to_string(),
        "bash".to_string(),
        "-c".to_string(),
        format!("{} \"$@\"", params.agent_command),
        "_".to_string(),
    ];

    let main_container = Container {
        name: "agent".to_string(),
        image: Some(params.image.clone()),
        args: Some(args),
        working_dir: Some("/workspace".to_string()),
        ports: Some(vec![ContainerPort {
            container_port: params.container_port,
            ..Default::default()
        }]),
        volume_mounts: Some(vec![VolumeMount {
            name: "workspace".to_string(),
            mount_path: "/workspace".to_string(),
            ..Default::default()
        }]),
        env: Some(env),
        startup_probe: Some(Probe {
            http_get: Some(HTTPGetAction {
                path: Some("/api/v1/health".to_string()),
                port: IntOrString::Int(params.container_port),
                ..Default::default()
            }),
            // 30 failures * 10s period = 300s for coop + Claude Code startup
            failure_threshold: Some(30),
            period_seconds: Some(10),
            ..Default::default()
        }),
        readiness_probe: Some(Probe {
            http_get: Some(HTTPGetAction {
                path: Some("/api/v1/health".to_string()),
                port: IntOrString::Int(params.container_port),
                ..Default::default()
            }),
            period_seconds: Some(5),
            ..Default::default()
        }),
        liveness_probe: Some(Probe {
            http_get: Some(HTTPGetAction {
                path: Some("/api/v1/health".to_string()),
                port: IntOrString::Int(params.container_port),
                ..Default::default()
            }),
            period_seconds: Some(30),
            ..Default::default()
        }),
        ..Default::default()
    };

    let init = if init_containers.is_empty() { None } else { Some(init_containers) };

    Pod {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(params.pod_name.clone()),
            namespace: Some(params.namespace.clone()),
            labels: Some(
                [
                    ("app".to_string(), "oj-agent".to_string()),
                    ("oj.dev/agent-id".to_string(), params.pod_name.clone()),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        },
        spec: Some(PodSpec {
            init_containers: init,
            containers: vec![main_container],
            volumes: Some(volumes),
            restart_policy: Some("Never".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn env_var(name: &str, value: &str) -> EnvVar {
    EnvVar { name: name.to_string(), value: Some(value.to_string()), ..Default::default() }
}

/// Build a git clone command for the init container.
pub(super) fn git_clone_command(repo: &str, branch: Option<&str>) -> Vec<String> {
    let mut cmd = vec!["git".to_string(), "clone".to_string()];
    if let Some(b) = branch {
        cmd.extend_from_slice(&["--branch".to_string(), b.to_string()]);
    }
    cmd.extend_from_slice(&[
        "--single-branch".to_string(),
        "--depth".to_string(),
        "1".to_string(),
        repo.to_string(),
        "/workspace".to_string(),
    ]);
    cmd
}
