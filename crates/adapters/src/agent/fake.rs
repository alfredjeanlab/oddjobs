// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Fake agent adapter for deterministic testing
#![cfg_attr(coverage_nightly, coverage(off))]

use super::{AgentAdapter, AgentError, AgentHandle, AgentReconnectConfig, AgentSpawnConfig};
use async_trait::async_trait;
use oj_core::{AgentId, AgentState, Event};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Recorded call to FakeAgentAdapter
#[derive(Debug, Clone)]
pub enum AgentCall {
    Spawn {
        agent_id: AgentId,
        command: String,
    },
    Reconnect {
        agent_id: AgentId,
        session_id: String,
    },
    Send {
        agent_id: AgentId,
        input: String,
    },
    Kill {
        agent_id: AgentId,
    },
    GetState {
        agent_id: AgentId,
    },
}

/// Fake agent adapter for testing
///
/// Allows programmatic control over agent behavior and records all calls.
#[derive(Clone)]
pub struct FakeAgentAdapter {
    inner: Arc<Mutex<FakeAgentState>>,
}

struct FakeAgentState {
    agents: HashMap<AgentId, FakeAgent>,
    calls: Vec<AgentCall>,
    spawn_error: Option<AgentError>,
    send_error: Option<AgentError>,
    kill_error: Option<AgentError>,
}

struct FakeAgent {
    state: AgentState,
    event_tx: Option<mpsc::Sender<Event>>,
    session_log_size: Option<u64>,
}

impl Default for FakeAgentAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeAgentAdapter {
    /// Create a new fake agent adapter
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeAgentState {
                agents: HashMap::new(),
                calls: Vec::new(),
                spawn_error: None,
                send_error: None,
                kill_error: None,
            })),
        }
    }

    /// Get all recorded calls
    pub fn calls(&self) -> Vec<AgentCall> {
        self.inner.lock().calls.clone()
    }

    /// Clear recorded calls
    pub fn clear_calls(&self) {
        self.inner.lock().calls.clear();
    }

    /// Set the state of an agent
    pub fn set_agent_state(&self, agent_id: &AgentId, state: AgentState) {
        let mut inner = self.inner.lock();
        if let Some(agent) = inner.agents.get_mut(agent_id) {
            agent.state = state;
        }
    }

    /// Emit a state change event for an agent
    pub async fn emit_state_change(&self, agent_id: &AgentId, state: AgentState) {
        let event_tx = {
            let inner = self.inner.lock();
            inner.agents.get(agent_id).and_then(|a| a.event_tx.clone())
        };

        if let Some(tx) = event_tx {
            let _ = tx
                .send(Event::from_agent_state(agent_id.clone(), state))
                .await;
        }
    }

    /// Set error to return on next spawn
    pub fn set_spawn_error(&self, error: AgentError) {
        self.inner.lock().spawn_error = Some(error);
    }

    /// Set error to return on next send
    pub fn set_send_error(&self, error: AgentError) {
        self.inner.lock().send_error = Some(error);
    }

    /// Set error to return on next kill
    pub fn set_kill_error(&self, error: AgentError) {
        self.inner.lock().kill_error = Some(error);
    }

    /// Set the session log size for an agent (for idle grace timer testing)
    pub fn set_session_log_size(&self, agent_id: &AgentId, size: Option<u64>) {
        let mut inner = self.inner.lock();
        if let Some(agent) = inner.agents.get_mut(agent_id) {
            agent.session_log_size = size;
        }
    }

    /// Check if an agent exists
    pub fn has_agent(&self, agent_id: &AgentId) -> bool {
        self.inner.lock().agents.contains_key(agent_id)
    }

    /// Get the number of active agents
    pub fn agent_count(&self) -> usize {
        self.inner.lock().agents.len()
    }
}

#[async_trait]
impl AgentAdapter for FakeAgentAdapter {
    async fn spawn(
        &self,
        config: AgentSpawnConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentError> {
        let mut inner = self.inner.lock();

        inner.calls.push(AgentCall::Spawn {
            agent_id: config.agent_id.clone(),
            command: config.command.clone(),
        });

        if let Some(error) = inner.spawn_error.take() {
            return Err(error);
        }

        inner.agents.insert(
            config.agent_id.clone(),
            FakeAgent {
                state: AgentState::Working,
                event_tx: Some(event_tx),
                session_log_size: None,
            },
        );

        Ok(AgentHandle::new(
            config.agent_id.clone(),
            config.agent_id.to_string(),
            config.workspace_path,
        ))
    }

    async fn reconnect(
        &self,
        config: AgentReconnectConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentError> {
        let mut inner = self.inner.lock();

        inner.calls.push(AgentCall::Reconnect {
            agent_id: config.agent_id.clone(),
            session_id: config.session_id.clone(),
        });

        if let Some(error) = inner.spawn_error.take() {
            return Err(error);
        }

        inner.agents.insert(
            config.agent_id.clone(),
            FakeAgent {
                state: AgentState::Working,
                event_tx: Some(event_tx),
                session_log_size: None,
            },
        );

        Ok(AgentHandle::new(
            config.agent_id.clone(),
            config.agent_id.to_string(),
            config.workspace_path,
        ))
    }

    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentError> {
        let mut inner = self.inner.lock();

        inner.calls.push(AgentCall::Send {
            agent_id: agent_id.clone(),
            input: input.to_string(),
        });

        if let Some(error) = inner.send_error.take() {
            return Err(error);
        }

        if !inner.agents.contains_key(agent_id) {
            return Err(AgentError::NotFound(agent_id.to_string()));
        }

        Ok(())
    }

    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentError> {
        let mut inner = self.inner.lock();

        inner.calls.push(AgentCall::Kill {
            agent_id: agent_id.clone(),
        });

        if let Some(error) = inner.kill_error.take() {
            return Err(error);
        }

        if inner.agents.remove(agent_id).is_none() {
            return Err(AgentError::NotFound(agent_id.to_string()));
        }

        Ok(())
    }

    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentError> {
        let mut inner = self.inner.lock();

        inner.calls.push(AgentCall::GetState {
            agent_id: agent_id.clone(),
        });

        inner
            .agents
            .get(agent_id)
            .map(|a| a.state.clone())
            .ok_or_else(|| AgentError::NotFound(agent_id.to_string()))
    }

    async fn session_log_size(&self, agent_id: &AgentId) -> Option<u64> {
        let inner = self.inner.lock();
        inner.agents.get(agent_id).and_then(|a| a.session_log_size)
    }
}

#[cfg(test)]
#[path = "fake_tests.rs"]
mod tests;
