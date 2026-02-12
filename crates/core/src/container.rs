// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Container configuration for running agents in Docker or Kubernetes.

use serde::{Deserialize, Serialize};

/// Container runtime configuration carried through effects.
///
/// When present on a `SpawnAgent` or `Shell` effect, the executor routes
/// to the container-aware adapter (Docker or Kubernetes) instead of
/// the local coop adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerConfig {
    /// Container image (e.g., "coop:claude")
    pub image: String,
}

impl ContainerConfig {
    pub fn new(image: impl Into<String>) -> Self {
        Self { image: image.into() }
    }
}
