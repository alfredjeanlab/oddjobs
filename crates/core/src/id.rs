// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! ID generation abstractions

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Generates unique identifiers
pub trait IdGen: Clone + Send + Sync {
    fn next(&self) -> String;
}

/// UUID-based ID generator for production use
#[derive(Clone, Default)]
pub struct UuidIdGen;

impl IdGen for UuidIdGen {
    fn next(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

/// Sequential ID generator for testing
#[derive(Clone)]
pub struct SequentialIdGen {
    prefix: String,
    counter: Arc<AtomicU64>,
}

impl SequentialIdGen {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            counter: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl Default for SequentialIdGen {
    fn default() -> Self {
        Self::new("id")
    }
}

impl IdGen for SequentialIdGen {
    fn next(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        format!("{}-{}", self.prefix, n)
    }
}

#[cfg(test)]
#[path = "id_tests.rs"]
mod tests;
