// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! WebSocket proxy bridge for agent attach.
//!
//! Connects to a coop instance's `/ws?mode=raw` endpoint and bridges
//! bidirectional byte streams between the CLI client and the coop terminal.

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

/// Bridge a raw byte stream (client) to a remote coop WebSocket via TCP.
///
/// Connects to `ws://{addr}/ws?mode=raw` and copies data bidirectionally.
/// Returns when either side disconnects.
pub async fn ws_proxy_bridge_tcp<R, W>(
    addr: &str,
    auth_token: Option<&str>,
    mut client_reader: R,
    mut client_writer: W,
) -> Result<(), BridgeError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let url = match auth_token {
        Some(token) => format!("ws://{}/ws?mode=raw&token={}", addr, token),
        None => format!("ws://{}/ws?mode=raw", addr),
    };

    let tcp_stream = tokio::net::TcpStream::connect(addr)
        .await
        .map_err(|e| BridgeError::Connect(format!("TCP connect to {}: {}", addr, e)))?;

    let (ws_stream, _) = tokio_tungstenite::client_async(&url, tcp_stream)
        .await
        .map_err(|e| BridgeError::Connect(format!("WS handshake with {}: {}", addr, e)))?;

    bridge_ws(ws_stream, &mut client_reader, &mut client_writer).await
}

/// Bidirectional bridge between a WebSocket stream and raw byte streams.
async fn bridge_ws<S, R, W>(
    ws_stream: tokio_tungstenite::WebSocketStream<S>,
    client_reader: &mut R,
    client_writer: &mut W,
) -> Result<(), BridgeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (mut ws_writer, mut ws_reader) = ws_stream.split();

    let mut buf = [0u8; 4096];
    loop {
        tokio::select! {
            // Client → WS
            result = client_reader.read(&mut buf) => {
                match result {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                        if ws_writer.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            // WS → Client
            msg = ws_reader.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if client_writer.write_all(text.as_bytes()).await.is_err() {
                            break;
                        }
                        if client_writer.flush().await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        if client_writer.write_all(&data).await.is_err() {
                            break;
                        }
                        if client_writer.flush().await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {} // Ping/Pong — ignore
                }
            }
        }
    }

    Ok(())
}

/// Errors from the WS proxy bridge.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("connection failed: {0}")]
    Connect(String),
}
