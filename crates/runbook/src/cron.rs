// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron definition for runbooks

use crate::RunDirective;
use serde::{Deserialize, Serialize};

/// A cron definition that runs a pipeline or agent on a timer interval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronDef {
    /// Cron name (injected from map key)
    #[serde(skip)]
    pub name: String,
    /// Interval duration string (e.g. "30m", "6h", "24h")
    pub interval: String,
    /// What to run (pipeline reference only)
    pub run: RunDirective,
    /// Maximum number of active pipelines this cron can have running
    /// simultaneously. Defaults to 1 (singleton). `None` means use default.
    #[serde(default)]
    pub concurrency: Option<u32>,
}
