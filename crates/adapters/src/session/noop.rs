// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! No-op session adapter for when session management is disabled.

use super::{SessionAdapter, SessionError};
use async_trait::async_trait;
use std::path::Path;

/// Session adapter that does nothing.
///
/// Used when agent spawning is disabled or in minimal deployments.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoOpSessionAdapter;

impl NoOpSessionAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SessionAdapter for NoOpSessionAdapter {
    async fn spawn(
        &self,
        _name: &str,
        _cwd: &Path,
        _cmd: &str,
        _env: &[(String, String)],
        _unset_env: &[String],
    ) -> Result<String, SessionError> {
        Ok("noop".to_string())
    }

    async fn send(&self, _id: &str, _input: &str) -> Result<(), SessionError> {
        Ok(())
    }

    async fn send_literal(&self, _id: &str, _text: &str) -> Result<(), SessionError> {
        Ok(())
    }

    async fn send_enter(&self, _id: &str) -> Result<(), SessionError> {
        Ok(())
    }

    async fn kill(&self, _id: &str) -> Result<(), SessionError> {
        Ok(())
    }

    async fn is_alive(&self, _id: &str) -> Result<bool, SessionError> {
        Ok(false)
    }

    async fn capture_output(&self, _id: &str, _lines: u32) -> Result<String, SessionError> {
        Ok(String::new())
    }

    async fn is_process_running(&self, _id: &str, _pattern: &str) -> Result<bool, SessionError> {
        Ok(false)
    }

    async fn get_exit_code(&self, _id: &str) -> Result<Option<i32>, SessionError> {
        Ok(None)
    }
}

#[cfg(test)]
#[path = "noop_tests.rs"]
mod tests;
