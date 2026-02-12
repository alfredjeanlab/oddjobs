// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use serde_json::json;

#[test]
fn test_migrate_same_version_is_noop() {
    let v1 = json!({"v": 1, "seq": 1, "state": {}});
    let registry = MigrationRegistry::new();
    let result = registry.migrate_to(v1.clone(), 1).unwrap();
    assert_eq!(result, v1);
}

#[test]
fn test_too_new_error() {
    let v99 = json!({"v": 99, "seq": 1, "state": {}});
    let registry = MigrationRegistry::new();
    assert!(matches!(registry.migrate_to(v99, 1), Err(MigrationError::TooNew(99, 1))));
}

#[test]
fn test_no_path_error() {
    // Try to migrate from v1 to v2 with no registered migrations
    let v1 = json!({"v": 1, "seq": 1, "state": {}});
    let registry = MigrationRegistry::new();
    assert!(matches!(registry.migrate_to(v1, 2), Err(MigrationError::NoPath(1, 2))));
}

/// Test migration with a mock migration
struct MockV1ToV2;

impl Migration for MockV1ToV2 {
    fn source_version(&self) -> u32 {
        1
    }
    fn target_version(&self) -> u32 {
        2
    }
    fn migrate(&self, snapshot: &mut Value) -> Result<(), MigrationError> {
        // Add a new field as part of migration
        if let Some(obj) = snapshot.as_object_mut() {
            obj.insert("migrated".into(), true.into());
        }
        Ok(())
    }
}

#[test]
fn test_migration_chain() {
    let mut registry = MigrationRegistry::new();
    registry.migrations.push(Box::new(MockV1ToV2));

    let v1 = json!({"v": 1, "seq": 42, "state": {}});
    let result = registry.migrate_to(v1, 2).unwrap();

    assert_eq!(result["v"], 2);
    assert_eq!(result["seq"], 42);
    assert_eq!(result["migrated"], true);
}
