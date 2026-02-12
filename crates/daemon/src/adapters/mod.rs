// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Adapters for external I/O

pub mod agent;
pub mod credential;
pub mod notify;
pub mod subprocess;
pub mod workspace;

pub use agent::attach_proxy::ws_proxy_bridge_tcp;
pub use agent::{
    AgentAdapter, AgentAdapterError, AgentConfig, AgentReconnectConfig, RuntimeRouter,
};
pub use notify::{DesktopNotifyAdapter, NotifyAdapter};
pub use workspace::{workspace_adapter, WorkspaceAdapter};

// Test support - only compiled for tests or when explicitly requested
#[cfg(test)]
pub use agent::{AgentCall, FakeAgentAdapter};
#[cfg(test)]
pub use notify::FakeNotifyAdapter;
