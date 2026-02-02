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
}
