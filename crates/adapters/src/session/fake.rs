// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Fake session adapter for testing
#![cfg_attr(coverage_nightly, coverage(off))]

use super::{SessionAdapter, SessionError};
use async_trait::async_trait;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Recorded session call
#[derive(Debug, Clone)]
pub enum SessionCall {
    Spawn {
        name: String,
        cwd: PathBuf,
        cmd: String,
        env: Vec<(String, String)>,
    },
    Send {
        id: String,
        input: String,
    },
    SendLiteral {
        id: String,
        text: String,
    },
    SendEnter {
        id: String,
    },
    Kill {
        id: String,
    },
    IsAlive {
        id: String,
    },
    CaptureOutput {
        id: String,
        lines: u32,
    },
    IsProcessRunning {
        id: String,
        pattern: String,
    },
}

/// Fake session state
#[derive(Debug, Clone)]
pub struct FakeSession {
    pub name: String,
    pub cwd: PathBuf,
    pub cmd: String,
    pub env: Vec<(String, String)>,
    pub output: Vec<String>,
    pub alive: bool,
    pub exit_code: Option<i32>,
    pub process_running: bool,
}

struct FakeSessionState {
    sessions: HashMap<String, FakeSession>,
    calls: Vec<SessionCall>,
    next_id: u64,
}

/// Fake session adapter for testing
#[derive(Clone)]
pub struct FakeSessionAdapter {
    inner: Arc<Mutex<FakeSessionState>>,
}

impl Default for FakeSessionAdapter {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeSessionState {
                sessions: HashMap::new(),
                calls: Vec::new(),
                next_id: 0,
            })),
        }
    }
}

impl FakeSessionAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all recorded calls
    pub fn calls(&self) -> Vec<SessionCall> {
        self.inner.lock().calls.clone()
    }

    /// Get a session by ID
    pub fn get_session(&self, id: &str) -> Option<FakeSession> {
        self.inner.lock().sessions.get(id).cloned()
    }

    /// Set session output
    pub fn set_output(&self, id: &str, output: Vec<String>) {
        if let Some(session) = self.inner.lock().sessions.get_mut(id) {
            session.output = output;
        }
    }

    /// Mark session as exited
    pub fn set_exited(&self, id: &str, exit_code: i32) {
        if let Some(session) = self.inner.lock().sessions.get_mut(id) {
            session.alive = false;
            session.exit_code = Some(exit_code);
        }
    }

    /// Set whether a process is running in the session
    pub fn set_process_running(&self, id: &str, running: bool) {
        if let Some(session) = self.inner.lock().sessions.get_mut(id) {
            session.process_running = running;
        }
    }

    /// Add a pre-existing session by ID (for testing liveness checks)
    pub fn add_session(&self, id: &str, alive: bool) {
        self.inner.lock().sessions.insert(
            id.to_string(),
            FakeSession {
                name: id.to_string(),
                cwd: PathBuf::new(),
                cmd: String::new(),
                env: Vec::new(),
                output: Vec::new(),
                alive,
                exit_code: None,
                process_running: alive,
            },
        );
    }
}

impl FakeSessionState {
    /// Record a call and check that the session exists, returning NotFound if not.
    fn record_and_check(&mut self, call: SessionCall, id: &str) -> Result<(), SessionError> {
        self.calls.push(call);
        if !self.sessions.contains_key(id) {
            return Err(SessionError::NotFound(id.to_string()));
        }
        Ok(())
    }
}

#[async_trait]
impl SessionAdapter for FakeSessionAdapter {
    async fn spawn(
        &self,
        name: &str,
        cwd: &Path,
        cmd: &str,
        env: &[(String, String)],
        _unset_env: &[String],
    ) -> Result<String, SessionError> {
        let mut inner = self.inner.lock();
        inner.next_id += 1;
        let id = format!("fake-{}", inner.next_id);
        inner.calls.push(SessionCall::Spawn {
            name: name.to_string(),
            cwd: cwd.to_path_buf(),
            cmd: cmd.to_string(),
            env: env.to_vec(),
        });
        inner.sessions.insert(
            id.clone(),
            FakeSession {
                name: name.to_string(),
                cwd: cwd.to_path_buf(),
                cmd: cmd.to_string(),
                env: env.to_vec(),
                output: Vec::new(),
                alive: true,
                exit_code: None,
                process_running: true,
            },
        );
        Ok(id)
    }

    async fn send(&self, id: &str, input: &str) -> Result<(), SessionError> {
        self.inner.lock().record_and_check(
            SessionCall::Send {
                id: id.to_string(),
                input: input.to_string(),
            },
            id,
        )
    }

    async fn send_literal(&self, id: &str, text: &str) -> Result<(), SessionError> {
        self.inner.lock().record_and_check(
            SessionCall::SendLiteral {
                id: id.to_string(),
                text: text.to_string(),
            },
            id,
        )
    }

    async fn send_enter(&self, id: &str) -> Result<(), SessionError> {
        self.inner
            .lock()
            .record_and_check(SessionCall::SendEnter { id: id.to_string() }, id)
    }

    async fn kill(&self, id: &str) -> Result<(), SessionError> {
        let mut inner = self.inner.lock();
        inner.calls.push(SessionCall::Kill { id: id.to_string() });
        if let Some(session) = inner.sessions.get_mut(id) {
            session.alive = false;
        }
        Ok(())
    }

    async fn is_alive(&self, id: &str) -> Result<bool, SessionError> {
        let mut inner = self.inner.lock();
        inner
            .calls
            .push(SessionCall::IsAlive { id: id.to_string() });
        Ok(inner.sessions.get(id).map(|s| s.alive).unwrap_or(false))
    }

    async fn capture_output(&self, id: &str, lines: u32) -> Result<String, SessionError> {
        let mut inner = self.inner.lock();
        inner.calls.push(SessionCall::CaptureOutput {
            id: id.to_string(),
            lines,
        });
        match inner.sessions.get(id) {
            Some(session) => {
                let start = session.output.len().saturating_sub(lines as usize);
                Ok(session.output[start..].join("\n"))
            }
            None => Err(SessionError::NotFound(id.to_string())),
        }
    }

    async fn is_process_running(&self, id: &str, pattern: &str) -> Result<bool, SessionError> {
        let mut inner = self.inner.lock();
        inner.calls.push(SessionCall::IsProcessRunning {
            id: id.to_string(),
            pattern: pattern.to_string(),
        });
        Ok(inner
            .sessions
            .get(id)
            .map(|s| s.process_running)
            .unwrap_or(false))
    }

    async fn get_exit_code(&self, id: &str) -> Result<Option<i32>, SessionError> {
        Ok(self.inner.lock().sessions.get(id).and_then(|s| s.exit_code))
    }
}

#[cfg(test)]
#[path = "fake_tests.rs"]
mod tests;
