// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Odd Jobs execution engine

mod activity_logger;
mod agent_logger;
mod agent_setup;
pub mod breadcrumb;
mod decision;
mod error;
mod executor;
pub(crate) mod lifecycle;
mod monitor;
mod runtime;
mod scheduler;
mod spawn;
mod steps;
mod time_fmt;
pub mod usage_metrics;
mod vars;
mod workspace;

#[cfg(test)]
pub(crate) mod test_helpers;

pub use agent_logger::AgentLogger;
pub use error::RuntimeError;
pub(crate) use monitor::ActionContext;
pub use runtime::{Runtime, RuntimeConfig, RuntimeDeps};
pub use usage_metrics::UsageMetricsCollector;
