// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! HTTP client for coop's Unix socket API.
//!
//! Sends HTTP/1.1 requests over Unix domain sockets. Reads responses using
//! Content-Length framing (does not depend on connection close for EOF).

use crate::adapters::agent::AgentAdapterError;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub async fn get(socket_path: &Path, path: &str) -> Result<String, AgentAdapterError> {
    let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n", path);
    timed_request(socket_path, &request).await
}

pub async fn post(socket_path: &Path, path: &str, body: &str) -> Result<String, AgentAdapterError> {
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        path,
        body.len(),
        body
    );
    timed_request(socket_path, &request).await
}

/// Connect, send, and read with a 5-second timeout covering the entire
/// operation (connect + write + read). Prevents hangs when coop's server
/// accepts the connection but doesn't send a response.
async fn timed_request(socket_path: &Path, request: &str) -> Result<String, AgentAdapterError> {
    tokio::time::timeout(Duration::from_secs(5), send_request(socket_path, request))
        .await
        .map_err(|_| AgentAdapterError::SessionError("HTTP request timed out".into()))?
}

async fn send_request(socket_path: &Path, request: &str) -> Result<String, AgentAdapterError> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| AgentAdapterError::SessionError(format!("connect failed: {}", e)))?;
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| AgentAdapterError::SessionError(format!("write failed: {}", e)))?;

    let mut reader = BufReader::new(&mut stream);
    read_http_response(&mut reader).await
}

/// Read and parse an HTTP/1.1 response from a buffered stream.
///
/// Shared by both Unix socket and TCP transports.
pub(in crate::adapters::agent) async fn read_http_response<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<String, AgentAdapterError> {
    // Read status line
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .await
        .map_err(|e| AgentAdapterError::SessionError(format!("read status failed: {}", e)))?;

    // Parse status code
    let status_code =
        status_line.split_whitespace().nth(1).and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);

    // Read headers, extract Content-Length (case-insensitive)
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| AgentAdapterError::SessionError(format!("read header failed: {}", e)))?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
        let line_lower = line.to_ascii_lowercase();
        if let Some(val) = line_lower.strip_prefix("content-length:") {
            content_length = val.trim().parse().unwrap_or(0);
        }
    }

    // Read body
    let body = if content_length > 0 {
        let mut buf = vec![0u8; content_length];
        reader
            .read_exact(&mut buf)
            .await
            .map_err(|e| AgentAdapterError::SessionError(format!("read body failed: {}", e)))?;
        String::from_utf8_lossy(&buf).into_owned()
    } else {
        String::new()
    };

    if status_code >= 400 {
        return Err(AgentAdapterError::SessionError(format!(
            "HTTP {}: {}",
            status_code,
            body.trim()
        )));
    }

    Ok(body)
}
