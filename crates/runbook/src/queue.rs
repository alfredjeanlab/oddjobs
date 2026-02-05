// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue definition for runbooks

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Type of queue backing
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueType {
    /// Queue backed by external shell commands (list/take)
    #[default]
    External,
    /// Queue backed by WAL-persisted state
    Persisted,
}

/// Retry configuration for persisted queues.
///
/// Controls automatic retry behavior when queue items fail.
/// When `attempts = 0` (the default), failed items go directly to `Dead`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Number of auto-retry attempts (0 = no auto-retry, items go straight to Dead)
    #[serde(default)]
    pub attempts: u32,
    /// Cooldown duration between retries (e.g. "30s", "5m"), default "0s"
    #[serde(default = "default_cooldown")]
    pub cooldown: String,
}

fn default_cooldown() -> String {
    "0s".into()
}

/// Report configuration for external queues.
///
/// Executes shell commands when items complete or fail, reporting back to external systems.
/// Optionally tracks completed/failed counts for visibility in `oj status` and `oj queue show`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReportConfig {
    /// Shell command to execute when an item completes successfully; supports {item.*} interpolation
    #[serde(default)]
    pub on_done: Option<String>,
    /// Shell command to execute when an item fails; supports {item.*} and {error} interpolation
    #[serde(default)]
    pub on_fail: Option<String>,
    /// Track and display completed item count in status output
    #[serde(default)]
    pub show_completed: bool,
    /// Track and display failed item count in status output
    #[serde(default)]
    pub show_failed: bool,
}

/// A queue definition for listing and claiming work items.
///
/// External queues use shell commands (`list`/`take`).
/// Persisted queues store items in `MaterializedState` via WAL events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueDef {
    /// Queue name (injected from map key)
    #[serde(skip)]
    pub name: String,
    /// Queue type: "external" (default) or "persisted"
    #[serde(rename = "type", default)]
    pub queue_type: QueueType,
    /// Shell command returning JSON array of items (external queues only)
    #[serde(default)]
    pub list: Option<String>,
    /// Shell command to claim an item; supports {item.*} interpolation (external queues only)
    #[serde(default)]
    pub take: Option<String>,
    /// Variable names for queue items (persisted queues only)
    #[serde(default)]
    pub vars: Vec<String>,
    /// Default values for variables (persisted queues only)
    #[serde(default)]
    pub defaults: HashMap<String, String>,
    /// Retry configuration (persisted queues only)
    #[serde(default)]
    pub retry: Option<RetryConfig>,
    /// Poll interval for external queues (e.g. "30s", "5m")
    /// When set, workers periodically check the queue at this interval
    #[serde(default)]
    pub poll: Option<String>,
    /// Report configuration for external queues (item completion/failure callbacks)
    #[serde(default)]
    pub report: Option<ReportConfig>,
}
