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
}
