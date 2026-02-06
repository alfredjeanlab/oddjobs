// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]
// Enable coverage(off) attribute for excluding test infrastructure
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

//! Adapters for external I/O

pub mod agent;
mod env;
pub mod notify;
pub mod session;
pub mod subprocess;
pub mod traced;

pub use agent::{
    extract_process_name, AgentAdapter, AgentAdapterError, AgentHandle, AgentReconnectConfig,
    AgentSpawnConfig, ClaudeAgentAdapter,
};
pub use notify::{DesktopNotifyAdapter, NoOpNotifyAdapter, NotifyAdapter};
pub use session::{NoOpSessionAdapter, SessionAdapter, TmuxAdapter};
pub use traced::{TracedAgent, TracedSession};

// Test support - only compiled for tests or when explicitly requested
#[cfg(any(test, feature = "test-support"))]
pub use agent::{AgentCall, FakeAgentAdapter};
#[cfg(any(test, feature = "test-support"))]
pub use notify::{FakeNotifyAdapter, NotifyCall};
#[cfg(any(test, feature = "test-support"))]
pub use session::{FakeSession, FakeSessionAdapter, SessionCall};
