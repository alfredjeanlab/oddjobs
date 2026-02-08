// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

//! Odd Jobs execution engine

mod activity_logger;
mod agent_logger;
pub mod breadcrumb;
mod decision_builder;
pub mod env;
mod error;
mod executor;
pub mod log_paths;
mod monitor;
mod runtime;
mod scheduler;
mod spawn;
mod steps;
mod time_fmt;
pub mod usage_metrics;
mod vars;
mod workspace;
mod workspace_fs;

#[cfg(test)]
pub(crate) mod test_helpers;

pub use activity_logger::{JobLogger, QueueLogger, WorkerLogger};
pub use agent_logger::AgentLogger;
pub use error::RuntimeError;
pub(crate) use monitor::ActionContext;
pub use runtime::{Runtime, RuntimeConfig, RuntimeDeps};
pub use usage_metrics::{MetricsHealth, UsageMetricsCollector};
