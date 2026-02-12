// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! HTTP client for coop's TCP API in Docker/Kubernetes containers.
//!
//! Same protocol as coop/http.rs but connects over TCP instead of Unix
//! sockets, with bearer-token authentication.

use crate::agent::coop::http::read_http_response;
use crate::agent::AgentAdapterError;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// HTTP GET with a bearer auth token.
pub async fn get_authed(addr: &str, path: &str, token: &str) -> Result<String, AgentAdapterError> {
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\n\r\n",
        path, token
    );
    timed_request(addr, &request).await
}

/// HTTP POST with a bearer auth token.
pub async fn post_authed(
    addr: &str,
    path: &str,
    body: &str,
    token: &str,
) -> Result<String, AgentAdapterError> {
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        path, token, body.len(), body
    );
    timed_request(addr, &request).await
}

async fn timed_request(addr: &str, request: &str) -> Result<String, AgentAdapterError> {
    tokio::time::timeout(Duration::from_secs(5), send_request(addr, request))
        .await
        .map_err(|_| AgentAdapterError::SessionError("HTTP request timed out".into()))?
}

async fn send_request(addr: &str, request: &str) -> Result<String, AgentAdapterError> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| AgentAdapterError::SessionError(format!("TCP connect failed: {}", e)))?;
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| AgentAdapterError::SessionError(format!("write failed: {}", e)))?;

    let mut reader = BufReader::new(&mut stream);
    read_http_response(&mut reader).await
}
