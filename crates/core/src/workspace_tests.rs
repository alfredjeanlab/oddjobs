// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn workspace_id_display() {
    let id = WorkspaceId::new("test-workspace");
    assert_eq!(id.to_string(), "test-workspace");
}

#[test]
fn workspace_id_equality() {
    let id1 = WorkspaceId::new("ws-1");
    let id2 = WorkspaceId::new("ws-1");
    let id3 = WorkspaceId::new("ws-2");

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
}

#[test]
fn workspace_id_from_str() {
    let id: WorkspaceId = "test".into();
    assert_eq!(id.as_str(), "test");
}

#[test]
fn workspace_id_serde() {
    let id = WorkspaceId::new("my-workspace");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"my-workspace\"");

    let parsed: WorkspaceId = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, id);
}

#[test]
fn workspace_status_display() {
    assert_eq!(WorkspaceStatus::Creating.to_string(), "creating");
    assert_eq!(WorkspaceStatus::Ready.to_string(), "ready");
    assert_eq!(
        WorkspaceStatus::InUse {
            by: "pipe-1".to_string()
        }
        .to_string(),
        "in_use(pipe-1)"
    );
    assert_eq!(WorkspaceStatus::Cleaning.to_string(), "cleaning");
    assert_eq!(
        WorkspaceStatus::Failed {
            reason: "disk full".to_string()
        }
        .to_string(),
        "failed: disk full"
    );
}

#[test]
fn workspace_status_serde() {
    let status = WorkspaceStatus::InUse {
        by: "test-pipeline".to_string(),
    };
    let json = serde_json::to_string(&status).unwrap();
    let parsed: WorkspaceStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, status);
}

#[test]
fn workspace_status_default() {
    let status = WorkspaceStatus::default();
    assert_eq!(status, WorkspaceStatus::Creating);
}
