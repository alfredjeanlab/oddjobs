// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker definition for runbooks

use serde::{Deserialize, Serialize};

fn default_concurrency() -> u32 {
    1
}

/// A worker definition that polls a queue and dispatches items to a pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerDef {
    /// Worker name (injected from map key)
    #[serde(skip)]
    pub name: String,
    /// Source reference: { queue = "name" }
    pub source: WorkerSource,
    /// Handler reference: { pipeline = "name" }
    pub handler: WorkerHandler,
    /// Max concurrent pipeline instances (default 1)
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
}

/// Source configuration for a worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerSource {
    /// Name of the queue to poll
    pub queue: String,
}

/// Handler configuration for a worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerHandler {
    /// Name of the pipeline to dispatch items to
    pub pipeline: String,
}
