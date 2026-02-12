// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Backward compatibility tests for Request deserialization.

use super::*;

#[test]
fn job_prune_failed_defaults_to_false() {
    let json = r#"{"type":"JobPrune","all":false,"dry_run":true}"#;
    let decoded: Request = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Request::JobPrune { all, failed, orphans, dry_run, project } => {
            assert!(!all);
            assert!(!failed);
            assert!(!orphans);
            assert!(dry_run);
            assert!(project.is_none());
        }
        _ => panic!("Expected JobPrune request"),
    }
}

#[test]
fn job_prune_orphans_defaults_to_false() {
    let json = r#"{"type":"JobPrune","all":true,"failed":false,"dry_run":false}"#;
    let decoded: Request = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Request::JobPrune { all, failed, orphans, dry_run, project } => {
            assert!(all);
            assert!(!failed);
            assert!(!orphans);
            assert!(!dry_run);
            assert!(project.is_none());
        }
        _ => panic!("Expected JobPrune request"),
    }
}

#[test]
fn job_prune_namespace_defaults_to_none() {
    let json = r#"{"type":"JobPrune","all":true,"failed":false,"orphans":false,"dry_run":false}"#;
    let decoded: Request = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Request::JobPrune { project, .. } => {
            assert!(project.is_none());
        }
        _ => panic!("Expected JobPrune request"),
    }
}

#[test]
fn workspace_prune_namespace_defaults_to_none() {
    let json = r#"{"type":"WorkspacePrune","all":false,"dry_run":true}"#;
    let decoded: Request = serde_json::from_str(json).expect("deserialize failed");
    match decoded {
        Request::WorkspacePrune { project, .. } => {
            assert!(project.is_none());
        }
        _ => panic!("Expected WorkspacePrune request"),
    }
}
