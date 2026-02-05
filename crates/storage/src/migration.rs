// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Snapshot migration system for schema evolution.
//!
//! Migrations transform snapshot JSON from one version to the next.
//! The registry chains migrations to reach the current version.

use serde_json::Value;
use thiserror::Error;

/// Errors that can occur during migration
#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("migration v{from}→v{to} failed: {reason}")]
    Failed { from: u32, to: u32, reason: String },
    #[error("no migration path from v{0} to v{1}")]
    NoPath(u32, u32),
    #[error("snapshot version {0} is newer than supported ({1})")]
    TooNew(u32, u32),
}

/// A migration from one snapshot version to the next.
pub trait Migration: Send + Sync {
    fn source_version(&self) -> u32;
    fn target_version(&self) -> u32;
    fn migrate(&self, snapshot: &mut Value) -> Result<(), MigrationError>;
}

/// Registry of migrations for upgrading snapshots.
pub struct MigrationRegistry {
    migrations: Vec<Box<dyn Migration>>,
}

impl MigrationRegistry {
    /// Create a new registry with all known migrations.
    pub fn new() -> Self {
        Self {
            migrations: Vec::new(),
        }
    }

    /// Migrate a snapshot to the target version.
    pub fn migrate_to(&self, mut snapshot: Value, target: u32) -> Result<Value, MigrationError> {
        let current = snapshot.get("v").and_then(|v| v.as_u64()).unwrap_or(1) as u32;

        if current == target {
            return Ok(snapshot);
        }
        if current > target {
            return Err(MigrationError::TooNew(current, target));
        }

        let mut version = current;
        while version < target {
            let migration = self
                .migrations
                .iter()
                .find(|m| m.source_version() == version)
                .ok_or(MigrationError::NoPath(version, target))?;

            migration.migrate(&mut snapshot)?;
            version = migration.target_version();

            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert("v".into(), version.into());
            }
        }
        Ok(snapshot)
    }
}

impl Default for MigrationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
