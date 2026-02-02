// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Notification adapters

mod noop;

pub use noop::NoOpNotifyAdapter;

// Test support - only compiled for tests or when explicitly requested
#[cfg(any(test, feature = "test-support"))]
mod fake;
#[cfg(any(test, feature = "test-support"))]
pub use fake::{FakeNotifyAdapter, NotifyCall};

use async_trait::async_trait;
use thiserror::Error;

/// Errors from notify operations
#[derive(Debug, Error)]
pub enum NotifyError {
    #[error("send failed: {0}")]
    SendFailed(String),
}

/// Adapter for sending notifications
#[async_trait]
pub trait NotifyAdapter: Clone + Send + Sync + 'static {
    /// Send a notification message
    async fn send(&self, channel: &str, message: &str) -> Result<(), NotifyError>;
}
