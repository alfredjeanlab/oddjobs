// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Wire format encoding/decoding for the IPC protocol.
//!
//! Wire format: 4-byte length prefix (big-endian) + JSON payload

use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

use super::{Request, Response};

/// Protocol errors
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Message too large: {size} bytes (max {max})")]
    MessageTooLarge { size: usize, max: usize },

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Timeout")]
    Timeout,
}

/// Maximum message size (200 MB)
pub const MAX_MESSAGE_SIZE: usize = 200 * 1024 * 1024;

/// Default IPC timeout
pub const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Protocol version (from Cargo.toml)
pub const PROTOCOL_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("BUILD_GIT_HASH"));

/// Encode a message to JSON bytes (without length prefix)
///
/// Use with `write_message()` which handles the length-prefix wire format.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, ProtocolError> {
    let json = serde_json::to_vec(msg)?;

    if json.len() > MAX_MESSAGE_SIZE {
        return Err(ProtocolError::MessageTooLarge {
            size: json.len(),
            max: MAX_MESSAGE_SIZE,
        });
    }

    Ok(json)
}

/// Decode a message from wire format
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtocolError> {
    Ok(serde_json::from_slice(bytes)?)
}

/// Read a length-prefixed message from an async reader
pub async fn read_message<R: tokio::io::AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>, ProtocolError> {
    // Read length prefix
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(ProtocolError::ConnectionClosed);
        }
        Err(e) => return Err(ProtocolError::Io(e)),
    }
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > MAX_MESSAGE_SIZE {
        return Err(ProtocolError::MessageTooLarge {
            size: len,
            max: MAX_MESSAGE_SIZE,
        });
    }

    // Read payload
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Write a length-prefixed message to an async writer
pub async fn write_message<W: tokio::io::AsyncWriteExt + Unpin>(
    writer: &mut W,
    data: &[u8],
) -> Result<(), ProtocolError> {
    let len = data.len();
    if len > MAX_MESSAGE_SIZE {
        return Err(ProtocolError::MessageTooLarge {
            size: len,
            max: MAX_MESSAGE_SIZE,
        });
    }

    writer.write_all(&(len as u32).to_be_bytes()).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a request with timeout
pub async fn read_request<R: tokio::io::AsyncReadExt + Unpin>(
    reader: &mut R,
    timeout: std::time::Duration,
) -> Result<Request, ProtocolError> {
    let bytes = tokio::time::timeout(timeout, read_message(reader))
        .await
        .map_err(|_| ProtocolError::Timeout)??;
    decode(&bytes)
}

/// Write a response with timeout
pub async fn write_response<W: tokio::io::AsyncWriteExt + Unpin>(
    writer: &mut W,
    response: &Response,
    timeout: std::time::Duration,
) -> Result<(), ProtocolError> {
    let data = encode(response)?;
    tokio::time::timeout(timeout, write_message(writer, &data))
        .await
        .map_err(|_| ProtocolError::Timeout)?
}
