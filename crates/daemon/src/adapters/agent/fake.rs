// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Fake agent adapter for deterministic testing
#![cfg_attr(coverage_nightly, coverage(off))]

use super::{AgentAdapter, AgentAdapterError, AgentConfig, AgentHandle, AgentReconnectConfig};
use async_trait::async_trait;
use oj_core::{AgentId, AgentState, Event, OwnerId};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Recorded call to FakeAgentAdapter
// NOTE(lifetime): fields used for test assertions via Debug
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum AgentCall {
    Spawn { agent_id: AgentId, command: String },
    Reconnect { agent_id: AgentId },
    Send { agent_id: AgentId, input: String },
    Kill { agent_id: AgentId },
    GetState { agent_id: AgentId },
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
    spawn_error: Option<AgentAdapterError>,
    send_error: Option<AgentAdapterError>,
    kill_error: Option<AgentAdapterError>,
}

struct FakeAgent {
    state: AgentState,
    owner: OwnerId,
    event_tx: Option<mpsc::Sender<Event>>,
    alive: bool,
    output: Vec<String>,
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

    /// Set the state of an agent
    pub fn set_agent_state(&self, agent_id: &AgentId, state: AgentState) {
        let mut inner = self.inner.lock();
        if let Some(agent) = inner.agents.get_mut(agent_id) {
            agent.state = state;
        }
    }

    /// Emit a state change event for an agent
    pub async fn emit_state_change(&self, agent_id: &AgentId, state: AgentState) {
        let (event_tx, owner) = {
            let inner = self.inner.lock();
            match inner.agents.get(agent_id) {
                Some(a) => (a.event_tx.clone(), a.owner.clone()),
                None => return,
            }
        };

        if let Some(tx) = event_tx {
            let _ = tx.send(Event::from_agent_state(agent_id.clone(), state, owner)).await;
        }
    }

    /// Set error to return on next spawn
    pub fn set_spawn_error(&self, error: AgentAdapterError) {
        self.inner.lock().spawn_error = Some(error);
    }

    /// Set error to return on next send
    pub fn set_send_error(&self, error: AgentAdapterError) {
        self.inner.lock().send_error = Some(error);
    }

    /// Set error to return on next kill
    pub fn set_kill_error(&self, error: AgentAdapterError) {
        self.inner.lock().kill_error = Some(error);
    }

    /// Check if an agent exists
    pub fn has_agent(&self, agent_id: &AgentId) -> bool {
        self.inner.lock().agents.contains_key(agent_id)
    }

    /// Set whether an agent is considered alive
    pub fn set_agent_alive(&self, agent_id: &AgentId, alive: bool) {
        let mut inner = self.inner.lock();
        if let Some(agent) = inner.agents.get_mut(agent_id) {
            agent.alive = alive;
        }
    }
}

impl FakeAgentState {
    fn register_agent(
        &mut self,
        agent_id: AgentId,
        owner: OwnerId,
        event_tx: mpsc::Sender<Event>,
    ) -> AgentHandle {
        self.agents.insert(
            agent_id.clone(),
            FakeAgent {
                state: AgentState::Working,
                owner,
                event_tx: Some(event_tx),
                alive: true,
                output: Vec::new(),
            },
        );
        AgentHandle::new(agent_id.clone())
    }
}

#[async_trait]
impl AgentAdapter for FakeAgentAdapter {
    async fn spawn(
        &self,
        config: AgentConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError> {
        let mut inner = self.inner.lock();
        inner.calls.push(AgentCall::Spawn {
            agent_id: config.agent_id.clone(),
            command: config.command.clone(),
        });
        if let Some(error) = inner.spawn_error.take() {
            return Err(error);
        }
        Ok(inner.register_agent(config.agent_id, config.owner, event_tx))
    }

    async fn reconnect(
        &self,
        config: AgentReconnectConfig,
        event_tx: mpsc::Sender<Event>,
    ) -> Result<AgentHandle, AgentAdapterError> {
        let mut inner = self.inner.lock();
        inner.calls.push(AgentCall::Reconnect { agent_id: config.agent_id.clone() });
        if let Some(error) = inner.spawn_error.take() {
            return Err(error);
        }
        Ok(inner.register_agent(config.agent_id, config.owner, event_tx))
    }

    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError> {
        let mut inner = self.inner.lock();
        inner.calls.push(AgentCall::Send { agent_id: agent_id.clone(), input: input.to_string() });
        if let Some(error) = inner.send_error.take() {
            return Err(error);
        }
        if !inner.agents.contains_key(agent_id) {
            return Err(AgentAdapterError::NotFound(agent_id.to_string()));
        }
        Ok(())
    }

    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError> {
        let mut inner = self.inner.lock();
        inner.calls.push(AgentCall::Kill { agent_id: agent_id.clone() });
        if let Some(error) = inner.kill_error.take() {
            return Err(error);
        }
        inner
            .agents
            .remove(agent_id)
            .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?;
        Ok(())
    }

    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError> {
        let mut inner = self.inner.lock();
        inner.calls.push(AgentCall::GetState { agent_id: agent_id.clone() });
        inner
            .agents
            .get(agent_id)
            .map(|a| a.state.clone())
            .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))
    }

    async fn last_message(&self, _agent_id: &AgentId) -> Option<String> {
        None
    }

    async fn resolve_stop(&self, _agent_id: &AgentId) {
        // no-op in fake adapter
    }

    async fn is_alive(&self, agent_id: &AgentId) -> bool {
        self.inner.lock().agents.get(agent_id).map(|a| a.alive).unwrap_or(false)
    }

    async fn capture_output(
        &self,
        agent_id: &AgentId,
        lines: u32,
    ) -> Result<String, AgentAdapterError> {
        let inner = self.inner.lock();
        let agent = inner
            .agents
            .get(agent_id)
            .ok_or_else(|| AgentAdapterError::NotFound(agent_id.to_string()))?;
        let start = agent.output.len().saturating_sub(lines as usize);
        Ok(agent.output[start..].join("\n"))
    }

    async fn fetch_transcript(&self, _agent_id: &AgentId) -> Result<String, AgentAdapterError> {
        Ok(String::new())
    }

    async fn fetch_usage(&self, _agent_id: &AgentId) -> Option<super::UsageData> {
        None
    }
}

#[cfg(test)]
#[path = "fake_tests.rs"]
mod tests;
