// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Custom error type that carries a process exit code.
//!
//! Commands return `ExitError` instead of calling `std::process::exit()`
//! directly, allowing `main()` to handle process termination.

use std::fmt;

#[derive(Debug)]
pub struct ExitError {
    pub code: i32,
    pub message: String,
}

impl ExitError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ExitError {}
