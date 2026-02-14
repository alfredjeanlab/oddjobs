// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Daemon-side handler for `AgentAttach`.
//!
//! Local agents: responds with `AgentAttachLocal` (socket path) so the CLI
//! can attach directly.
//!
//! Remote agents (Docker/K8S): responds with `AgentAttachReady` then proxies
//! raw bytes between the CLI connection and the agent's coop WebSocket.

use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{info, warn};

use crate::protocol::{self, Response};

use super::{ConnectionError, ListenCtx};

/// Handle an `AgentAttach` request.
///
/// For remote agents, this is a connection-upgrading request: after
/// `AgentAttachReady`, the connection becomes a raw bidirectional byte
/// stream proxied to the agent's coop WebSocket.
///
/// For local agents, responds with `AgentAttachLocal` and returns.
pub(super) async fn handle_agent_attach<R, W>(
    agent_id: &str,
    _token: Option<&str>,
    mut reader: R,
    mut writer: W,
    ctx: &ListenCtx,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    // Resolve agent ID prefix → full ID
    let full_agent_id = {
        let state = ctx.state.lock();
        resolve_agent_id(agent_id, &state)
    };

    let full_agent_id = match full_agent_id {
        Some(id) => id,
        None => {
            let resp = Response::Error { message: format!("agent not found: {}", agent_id) };
            protocol::write_response(&mut writer, &resp, super::ipc_timeout()).await?;
            return Ok(());
        }
    };

    // Resolve coop connection info — local socket or remote TCP
    let coop_info = ctx.agent.get_coop_host(&full_agent_id);

    match coop_info {
        Some(info) if info.remote => {
            // Remote agent — proxy through daemon
            // Strip http:// prefix to get host:port for TCP connect
            let addr = info.url.strip_prefix("http://").unwrap_or(&info.url);
            let token =
                if info.auth_token.is_empty() { None } else { Some(info.auth_token.as_str()) };

            info!(agent_id = %full_agent_id, addr = %addr, "proxying remote attach");

            let resp = Response::AgentAttachReady { id: full_agent_id.as_str().to_string() };
            protocol::write_response(&mut writer, &resp, super::ipc_timeout()).await?;

            match crate::adapters::ws_proxy_bridge_tcp(addr, token, &mut reader, &mut writer).await
            {
                Ok(()) => {
                    info!(agent_id = %full_agent_id, "attach proxy disconnected cleanly");
                }
                Err(e) => {
                    warn!(agent_id = %full_agent_id, error = %e, "attach proxy error");
                }
            }
        }
        _ => {
            // Local agent — return socket path for direct attach
            let socket_path = match coop_info {
                Some(info) => info.url,
                None => oj_core::agent_dir(&ctx.state_dir, full_agent_id.as_str())
                    .join("coop.sock")
                    .to_string_lossy()
                    .to_string(),
            };

            info!(agent_id = %full_agent_id, "local attach via {}", socket_path);

            let resp =
                Response::AgentAttachLocal { id: full_agent_id.as_str().to_string(), socket_path };
            protocol::write_response(&mut writer, &resp, super::ipc_timeout()).await?;
        }
    }

    Ok(())
}

/// Resolve an agent ID prefix to a full agent ID.
///
/// Checks the unified agents map first, then falls back to crew.
fn resolve_agent_id(
    prefix: &str,
    state: &crate::storage::MaterializedState,
) -> Option<oj_core::AgentId> {
    // Unified agents map
    state
        .agents
        .values()
        .find(|a| a.agent_id.starts_with(prefix))
        .map(|a| oj_core::AgentId::from_string(&a.agent_id))
        .or_else(|| {
            // Crew fallback — match by crew ID, return agent UUID
            state
                .crew
                .values()
                .find(|r| r.id.starts_with(prefix))
                .and_then(|r| r.agent_id.as_ref().map(oj_core::AgentId::from_string))
        })
}
