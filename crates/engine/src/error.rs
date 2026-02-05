// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Error types for the engine runtime

use crate::executor::ExecuteError;
use thiserror::Error;

/// Errors that can occur in the runtime
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("execute error: {0}")]
    Execute(#[from] ExecuteError),
    #[error("job not found: {0}")]
    JobNotFound(String),
    #[error("command not found: {0}")]
    CommandNotFound(String),
    #[error("job definition not found: {0}")]
    JobDefNotFound(String),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("prompt error for agent {agent}: {message}")]
    PromptError { agent: String, message: String },
    #[error("invalid run directive for {context}: {directive}")]
    InvalidRunDirective { context: String, directive: String },
    #[error("failed to load runbook: {0}")]
    RunbookLoadError(String),
    #[error("invalid format: {0}")]
    InvalidFormat(String),
    #[error("step not found: {0}")]
    StepNotFound(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("worker not found: {0}")]
    WorkerNotFound(String),
    #[error("cron not found: {0}")]
    CronNotFound(String),
    #[error("shell error: {0}")]
    ShellError(String),
}
