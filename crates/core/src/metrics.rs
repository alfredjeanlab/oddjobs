// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Metrics health type shared between engine and wire crates.

use serde::{Deserialize, Serialize};

/// Health information shared with the listener for `oj status`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsHealth {
    pub last_collection_ms: u64,
    pub agents_tracked: usize,
    pub last_error: Option<String>,
    pub ghost_agents: Vec<String>,
}
