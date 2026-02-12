// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! WebSocket event bridge for containerized agents.
//!
//! Same protocol as coop/ws.rs but connects over TCP instead of Unix sockets.
//! Subscribes to coop's state+messages stream inside the container and
//! translates events into oddjobs `Event` values.

use super::http;
use crate::adapters::agent::coop::ws as coop_ws;
use crate::adapters::agent::log_entry::AgentLogMessage;
use futures_util::StreamExt;
use oj_core::{AgentId, Event, OwnerId};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

/// Background task that subscribes to a containerized coop's WebSocket stream and emits events.
pub(crate) async fn event_bridge(
    addr: String,
    auth_token: String,
    agent_id: AgentId,
    owner: OwnerId,
    event_tx: mpsc::Sender<Event>,
    shutdown_rx: oneshot::Receiver<()>,
    log_entry_tx: Option<mpsc::Sender<AgentLogMessage>>,
) {
    let mut shutdown_rx = shutdown_rx;

    // Connect WebSocket over TCP
    let ws_stream = match connect_ws(&addr, &auth_token).await {
        Some(s) => {
            tracing::info!(%agent_id, %addr, "docker ws bridge connected");
            s
        }
        None => {
            tracing::warn!(%agent_id, %addr, "docker ws bridge: connection failed, emitting AgentGone");
            let _ = event_tx
                .send(Event::AgentGone {
                    id: agent_id.clone(),
                    owner: owner.clone(),
                    exit_code: None,
                })
                .await;
            return;
        }
    };

    let (_, mut read) = ws_stream.split();

    // Catch up: poll current state via HTTP before WS events start flowing.
    if let Ok(Some(event)) = tokio::time::timeout(
        Duration::from_secs(3),
        poll_initial_state(&addr, &auth_token, &agent_id, &owner),
    )
    .await
    {
        tracing::info!(%agent_id, ?event, "docker ws bridge: initial state event");
        let _ = event_tx.send(event).await;
    }

    tracing::info!(%agent_id, "docker ws bridge: entering event loop");

    let mut last_user_timestamp: Option<String> = None;

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match coop_ws::parse_ws_event(&text, &agent_id, &owner) {
                            coop_ws::WsParseResult::Event(event) => {
                                tracing::info!(%agent_id, ?event, "docker ws bridge: emitting event");
                                let _ = event_tx.send(*event).await;
                            }
                            coop_ws::WsParseResult::None => {}
                        }

                        // Extract log entries from message:raw events
                        if let Some(ref tx) = log_entry_tx {
                            if let Some(entries) = coop_ws::extract_log_entries_from_ws(&text, &mut last_user_timestamp) {
                                if !entries.is_empty() {
                                    let _ = tx.send((agent_id.clone(), entries)).await;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        tracing::info!(%agent_id, ?frame, "docker ws bridge: received close frame");
                        let _ = event_tx.send(Event::AgentGone {
                            id: agent_id.clone(),
                            owner: owner.clone(),
                            exit_code: None,
                        }).await;
                        break;
                    }
                    None => {
                        tracing::info!(%agent_id, "docker ws bridge: stream ended");
                        let _ = event_tx.send(Event::AgentGone {
                            id: agent_id.clone(),
                            owner: owner.clone(),
                            exit_code: None,
                        }).await;
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::warn!(%agent_id, %e, "docker ws bridge: error");
                        let _ = event_tx.send(Event::AgentGone {
                            id: agent_id.clone(),
                            owner: owner.clone(),
                            exit_code: None,
                        }).await;
                        break;
                    }
                    _ => {} // Ping/Pong/Binary — ignore
                }
            }
            _ = &mut shutdown_rx => {
                break;
            }
        }
    }
}

/// Connect a WebSocket over TCP to a containerized coop's state subscription endpoint.
async fn connect_ws(
    addr: &str,
    auth_token: &str,
) -> Option<tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>> {
    // Retry while the container starts up
    let stream = {
        let mut stream = None;
        for i in 0..20 {
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            if let Ok(s) = tokio::net::TcpStream::connect(addr).await {
                stream = Some(s);
                break;
            }
        }
        if stream.is_none() {
            tracing::warn!(%addr, "docker ws connect: failed after 20 attempts");
        }
        stream?
    };

    // WebSocket handshake with auth token in query string
    let uri = format!("ws://{}/ws?subscribe=state,messages&token={}", addr, auth_token);
    match tokio_tungstenite::client_async(&uri, stream).await {
        Ok((ws, _)) => Some(ws),
        Err(e) => {
            tracing::warn!(%addr, error = %e, "docker ws connect: WebSocket handshake failed");
            None
        }
    }
}

/// Poll coop's HTTP endpoint in the container for the current state.
async fn poll_initial_state(
    addr: &str,
    auth_token: &str,
    agent_id: &AgentId,
    owner: &OwnerId,
) -> Option<Event> {
    tracing::info!(%agent_id, "docker poll_initial_state: sending HTTP GET /api/v1/agent");
    let body = match http::get_authed(addr, "/api/v1/agent", auth_token).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(%agent_id, %e, "docker poll_initial_state: HTTP request failed");
            return None;
        }
    };
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    coop_ws::map_initial_state(&json, agent_id, owner)
}
