// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Push-specific data validation.
//!
//! Handles JSON object validation, required field checks, default
//! application, and deduplication for queue push operations.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::Mutex;

use oj_core::scoped_name;
use oj_storage::MaterializedState;

use crate::protocol::Response;

/// Validate that the push data is a JSON object. Returns the object map.
pub(super) fn validate_queue_data(
    data: &serde_json::Value,
) -> Result<&serde_json::Map<String, serde_json::Value>, Response> {
    data.as_object().ok_or_else(|| Response::Error {
        message: "data must be a JSON object".to_string(),
    })
}

/// Check that all required fields are present in the data.
pub(super) fn validate_required_fields(
    queue_def: &oj_runbook::QueueDef,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), Response> {
    let data_keys: HashSet<&str> = obj.keys().map(|k| k.as_str()).collect();
    let missing: Vec<&str> = queue_def
        .vars
        .iter()
        .filter(|v| !data_keys.contains(v.as_str()) && !queue_def.defaults.contains_key(v.as_str()))
        .map(|v| v.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(Response::Error {
            message: format!("missing required fields: {}", missing.join(", ")),
        });
    }
    Ok(())
}

/// Build the final data HashMap, applying defaults for missing optional fields.
pub(super) fn apply_defaults(
    queue_def: &oj_runbook::QueueDef,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, String> {
    let mut final_data: HashMap<String, String> = obj
        .iter()
        .map(|(k, v)| {
            let s = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (k.clone(), s)
        })
        .collect();
    for (key, default_val) in &queue_def.defaults {
        if !final_data.contains_key(key) {
            final_data.insert(key.clone(), default_val.clone());
        }
    }
    final_data
}

/// Find a pending or active item with the same data (deduplication).
///
/// Returns the item ID if a duplicate is found, or `None` if no duplicate exists.
pub(super) fn find_duplicate_item(
    state: &Arc<Mutex<MaterializedState>>,
    namespace: &str,
    queue_name: &str,
    data: &HashMap<String, String>,
) -> Option<String> {
    let st = state.lock();
    let key = scoped_name(namespace, queue_name);
    st.queue_items.get(&key).and_then(|items| {
        items
            .iter()
            .find(|i| {
                (i.status == oj_storage::QueueItemStatus::Pending
                    || i.status == oj_storage::QueueItemStatus::Active)
                    && i.data == *data
            })
            .map(|i| i.id.clone())
    })
}
