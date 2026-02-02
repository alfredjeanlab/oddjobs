// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Fake notification adapter for testing
#![cfg_attr(coverage_nightly, coverage(off))]

use super::{NotifyAdapter, NotifyError};
use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::Arc;

/// Recorded notification
#[derive(Debug, Clone)]
pub struct NotifyCall {
    pub title: String,
    pub message: String,
}

struct FakeNotifyState {
    calls: Vec<NotifyCall>,
}

/// Fake notification adapter for testing
#[derive(Clone)]
pub struct FakeNotifyAdapter {
    inner: Arc<Mutex<FakeNotifyState>>,
}

impl Default for FakeNotifyAdapter {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeNotifyState { calls: Vec::new() })),
        }
    }
}

impl FakeNotifyAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all recorded notifications
    pub fn calls(&self) -> Vec<NotifyCall> {
        self.inner.lock().calls.clone()
    }
}

#[async_trait]
impl NotifyAdapter for FakeNotifyAdapter {
    async fn notify(&self, title: &str, message: &str) -> Result<(), NotifyError> {
        self.inner.lock().calls.push(NotifyCall {
            title: title.to_string(),
            message: message.to_string(),
        });
        Ok(())
    }
}

#[cfg(test)]
#[path = "fake_tests.rs"]
mod tests;
