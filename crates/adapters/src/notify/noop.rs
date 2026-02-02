// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! No-op notification adapter.

use super::{NotifyAdapter, NotifyError};
use async_trait::async_trait;

/// Notification adapter that silently discards all notifications.
///
/// Used when notifications are disabled or not yet configured.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoOpNotifyAdapter;

impl NoOpNotifyAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NotifyAdapter for NoOpNotifyAdapter {
    async fn notify(&self, _title: &str, _message: &str) -> Result<(), NotifyError> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "noop_tests.rs"]
mod tests;
