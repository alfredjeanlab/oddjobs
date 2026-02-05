// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for the shell executor.

use super::*;

mod basic;
mod builder;
mod errors;
mod expansion;
mod groups;
mod jobs;
mod redirections;
mod variables;

/// Create a default executor for tests.
pub(crate) fn executor() -> ShellExecutor {
    ShellExecutor::new()
}

/// Sync wrapper for async execution in parameterized tests.
pub(crate) fn run_async<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Runtime::new().unwrap().block_on(f)
}
