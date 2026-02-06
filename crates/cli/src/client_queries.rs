// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Query and command methods for DaemonClient.

#[path = "client_queries_job.rs"]
mod job;

#[path = "client_queries_agent.rs"]
mod agent;

#[path = "client_queries_worker.rs"]
mod worker;

#[path = "client_queries_queue.rs"]
mod queue;

pub use job::RunCommandResult;
pub use queue::{QueuePushResult, QueueRetryResult};
pub use worker::{StartResult, StopResult};
