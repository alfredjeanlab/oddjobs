// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Container configuration for running agents and jobs in Docker/Kubernetes.
//!
//! Supports two forms in runbooks:
//!
//! Short form (image only):
//! ```hcl
//! container = "coop:claude"
//! ```
//!
//! Block form (image with future options):
//! ```hcl
//! container {
//!   image = "coop:claude"
//! }
//! ```

use serde::{Deserialize, Deserializer, Serialize};

/// Container configuration for an agent or job.
///
/// Short form: `container = "image"` — just an image name.
/// Block form: `container { image = "..." }` — image with options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainerConfig {
    /// Container image (e.g., "coop:claude")
    pub image: String,
}

impl ContainerConfig {
    pub fn new(image: impl Into<String>) -> Self {
        Self { image: image.into() }
    }
}

impl<'de> Deserialize<'de> for ContainerConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Short(String),
            Block { image: String },
        }

        match Helper::deserialize(deserializer)? {
            Helper::Short(image) => Ok(ContainerConfig { image }),
            Helper::Block { image } => Ok(ContainerConfig { image }),
        }
    }
}

#[cfg(test)]
#[path = "container_tests.rs"]
mod tests;
