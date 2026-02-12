// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent spawn logic — process creation, workspace preparation, readiness polling.

use super::http;
use super::LocalAdapter;
use crate::adapters::agent::{AgentAdapterError, AgentConfig, AgentHandle};
use oj_core::Event;
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;

/// Spawn a coop agent process and wait for it to become ready.
pub(super) async fn execute(
    adapter: &LocalAdapter,
    config: AgentConfig,
    event_tx: mpsc::Sender<Event>,
) -> Result<AgentHandle, AgentAdapterError> {
    // Precondition: cwd must exist if specified
    if let Some(ref cwd) = config.cwd {
        if !cwd.exists() {
            return Err(AgentAdapterError::SpawnFailed(format!(
                "working directory does not exist: {}",
                cwd.display()
            )));
        }
    }

    // Prepare workspace directory
    prepare_workspace(&config.workspace_path)
        .await
        .map_err(|e| AgentAdapterError::WorkspaceError(e.to_string()))?;

    // Determine paths
    let cwd = config.cwd.clone().unwrap_or_else(|| config.workspace_path.clone());
    let agent_dir = oj_core::agent_dir(&adapter.state_dir, config.agent_id.as_str());
    std::fs::create_dir_all(&agent_dir).map_err(|e| {
        AgentAdapterError::SpawnFailed(format!("failed to create agent dir: {}", e))
    })?;
    let socket_path = agent_dir.join("coop.sock");
    let agent_config_path = agent_dir.join("agent-config.json");

    // Remove stale socket from previous run
    let _ = std::fs::remove_file(&socket_path);

    // Build coop command wrapping the claude command
    let command = crate::adapters::agent::augment_command_for_skip_permissions(&config.command);
    let mut coop_cmd = tokio::process::Command::new("coop");
    coop_cmd.arg("--agent").arg("claude").arg("--socket").arg(&socket_path);

    // Pass agent-config if the engine wrote one (contains settings + stop config)
    if agent_config_path.exists() {
        coop_cmd.arg("--agent-config").arg(&agent_config_path);
    }

    // Delegate session resume to coop (discovers JSONL, appends --resume to claude args).
    // Pass the workspace path as the resume hint so coop can find the session log.
    if config.resume {
        coop_cmd.arg("--resume").arg(&config.workspace_path);
    }

    // Use `"$@"` so coop can inject extra CLI args (e.g. --settings, --session-id)
    // that get forwarded to the wrapped command. The `_` becomes $0, and coop's
    // extra_args become $1..$N which "$@" expands.
    coop_cmd
        .arg("--")
        .arg("bash")
        .arg("-c")
        .arg(format!("{command} \"$@\""))
        .arg("_")
        .current_dir(&cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Forward environment variables
    for (key, value) in &config.env {
        coop_cmd.env(key, value);
    }
    for key in &config.unset_env {
        coop_cmd.env_remove(key);
    }

    // Apply faster coop timings for orchestrated agents (mirroring coop's
    // Config::test values). The production defaults (3s screen poll, 10s
    // process poll, etc.) are too slow for responsive lifecycle handling.
    // Each can still be overridden via the corresponding env var.
    let coop_defaults: &[(&str, &str)] = &[
        ("COOP_SCREEN_POLL_MS", "50"),
        ("COOP_SCREEN_DEBOUNCE_MS", "10"),
        ("COOP_LOG_POLL_MS", "50"),
        ("COOP_PROCESS_POLL_MS", "50"),
        ("COOP_REAP_POLL_MS", "10"),
        ("COOP_INPUT_DELAY_MS", "10"),
        ("COOP_INPUT_DELAY_PER_BYTE_MS", "0"),
        ("COOP_INPUT_DELAY_MAX_MS", "50"),
        ("COOP_NUDGE_TIMEOUT_MS", "100"),
        ("COOP_GROOM_DISMISS_DELAY_MS", "50"),
        ("COOP_DRAIN_TIMEOUT_MS", "100"),
        ("COOP_SHUTDOWN_TIMEOUT_MS", "100"),
    ];
    for (key, val) in coop_defaults {
        if std::env::var(key).is_err() {
            coop_cmd.env(key, val);
        }
    }

    // Spawn coop process
    let child = coop_cmd
        .spawn()
        .map_err(|e| AgentAdapterError::SpawnFailed(format!("failed to spawn coop: {}", e)))?;

    // Spawn reaper task to prevent zombie processes
    let reaper_agent_id = config.agent_id.clone();
    tokio::spawn(async move {
        let child = child;
        match child.wait_with_output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!(
                    agent_id = %reaper_agent_id,
                    exit_status = %output.status,
                    stdout = %stdout,
                    stderr = %stderr,
                    "coop process exited"
                );
            }
            Err(e) => {
                tracing::error!(agent_id = %reaper_agent_id, error = %e, "failed to wait on coop process");
            }
        }
    });

    tracing::info!(
        agent_id = %config.agent_id,
        socket_path = %socket_path.display(),
        "coop process spawned"
    );

    // Wait for coop to be ready
    wait_for_ready(&socket_path).await?;

    // Register and start state polling
    Ok(adapter.register_agent(
        config.agent_id,
        socket_path,
        config.workspace_path,
        event_tx,
        config.owner,
    ))
}

/// Wait for coop to become ready (health check succeeds).
async fn wait_for_ready(socket_path: &Path) -> Result<(), AgentAdapterError> {
    let poll_ms: u64 =
        std::env::var("OJ_COOP_READY_POLL_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(50);
    let max_attempts: usize =
        std::env::var("OJ_COOP_READY_ATTEMPTS").ok().and_then(|v| v.parse().ok()).unwrap_or(200); // 200 * 50ms = 10s default

    for i in 0..max_attempts {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(poll_ms)).await;
        }
        if let Ok(body) = http::get(socket_path, "/api/v1/health").await {
            tracing::info!(
                socket_path = %socket_path.display(),
                body = %body,
                attempt = i,
                "coop health check succeeded"
            );
            return Ok(());
        }
    }
    let socket_exists = socket_path.exists();
    tracing::error!(
        socket_path = %socket_path.display(),
        socket_exists = socket_exists,
        max_attempts,
        poll_ms,
        "coop failed to become ready"
    );
    Err(AgentAdapterError::SpawnFailed(format!(
        "coop failed to become ready within {}s",
        (max_attempts as u64 * poll_ms) / 1000
    )))
}

/// Prepare workspace directory for agent execution.
///
/// Settings are now passed via coop's `--agent-config` file rather than
/// being copied into the workspace as `settings.local.json`.
async fn prepare_workspace(workspace_path: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(workspace_path).await?;
    Ok(())
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
