// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

// Allow panic!/unwrap/expect in test code
#![cfg_attr(test, allow(clippy::panic))]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

//! Odd Jobs execution engine

mod agent_logger;
pub mod breadcrumb;
mod decision_builder;
pub mod env;
mod error;
mod executor;
mod job_logger;
pub mod log_paths;
mod monitor;
mod queue_logger;
mod runtime;
mod scheduler;
mod spawn;
mod steps;
mod time_fmt;
mod worker_logger;
mod workspace;

pub use agent_logger::AgentLogger;
pub use error::RuntimeError;
pub use runtime::{Runtime, RuntimeConfig, RuntimeDeps};
