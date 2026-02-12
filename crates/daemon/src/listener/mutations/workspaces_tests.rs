// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use oj_core::Event;

use crate::protocol::Response;

use super::super::PruneFlags;
use super::workspace_prune_inner;
use crate::listener::test_ctx;
use crate::listener::test_fixtures::{make_job_ns, make_workspace};

#[tokio::test]
async fn workspace_prune_emits_deleted_events_for_fs_workspaces() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());
    let cancel = CancellationToken::new();

    let workspaces_dir = dir.path().join("workspaces");
    std::fs::create_dir_all(&workspaces_dir).unwrap();

    let ws1_path = workspaces_dir.join("ws-test-1");
    let ws2_path = workspaces_dir.join("ws-test-2");
    std::fs::create_dir_all(&ws1_path).unwrap();
    std::fs::create_dir_all(&ws2_path).unwrap();

    {
        let mut s = ctx.state.lock();
        s.workspaces
            .insert("ws-test-1".to_string(), make_workspace("ws-test-1", ws1_path.clone(), None));
        s.workspaces
            .insert("ws-test-2".to_string(), make_workspace("ws-test-2", ws2_path.clone(), None));
    }

    let flags = PruneFlags { all: true, dry_run: false, project: None };
    let result =
        workspace_prune_inner(&ctx.state, &ctx.event_bus, &flags, &workspaces_dir, &cancel).await;

    match result {
        Ok(Response::WorkspacesPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 2, "should prune both workspaces");
            assert_eq!(skipped, 0);
            let ids: Vec<&str> = pruned.iter().map(|ws| ws.id.as_str()).collect();
            assert!(ids.contains(&"ws-test-1"));
            assert!(ids.contains(&"ws-test-2"));
        }
        other => panic!("expected WorkspacesPruned, got: {:?}", other),
    }

    assert!(!ws1_path.exists(), "ws-test-1 directory should be removed");
    assert!(!ws2_path.exists(), "ws-test-2 directory should be removed");

    {
        let mut s = ctx.state.lock();
        s.apply_event(&Event::WorkspaceDeleted { id: oj_core::WorkspaceId::new("ws-test-1") });
        s.apply_event(&Event::WorkspaceDeleted { id: oj_core::WorkspaceId::new("ws-test-2") });
        assert!(!s.workspaces.contains_key("ws-test-1"));
        assert!(!s.workspaces.contains_key("ws-test-2"));
    }
}

#[tokio::test]
async fn workspace_prune_removes_orphaned_state_entries() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());
    let cancel = CancellationToken::new();

    let workspaces_dir = dir.path().join("workspaces");
    std::fs::create_dir_all(&workspaces_dir).unwrap();

    {
        let mut s = ctx.state.lock();
        s.workspaces.insert(
            "ws-orphan-1".to_string(),
            make_workspace("ws-orphan-1", workspaces_dir.join("ws-orphan-1"), None),
        );
        s.workspaces.insert(
            "ws-orphan-2".to_string(),
            make_workspace("ws-orphan-2", workspaces_dir.join("ws-orphan-2"), None),
        );
    }

    let flags = PruneFlags { all: true, dry_run: false, project: None };
    let result =
        workspace_prune_inner(&ctx.state, &ctx.event_bus, &flags, &workspaces_dir, &cancel).await;

    match result {
        Ok(Response::WorkspacesPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 2, "should prune orphaned state entries");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected WorkspacesPruned, got: {:?}", other),
    }
}

#[tokio::test]
async fn workspace_prune_dry_run_does_not_delete() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());
    let cancel = CancellationToken::new();

    let workspaces_dir = dir.path().join("workspaces");
    std::fs::create_dir_all(&workspaces_dir).unwrap();

    let ws_path = workspaces_dir.join("ws-keep");
    std::fs::create_dir_all(&ws_path).unwrap();

    {
        let mut s = ctx.state.lock();
        s.workspaces
            .insert("ws-keep".to_string(), make_workspace("ws-keep", ws_path.clone(), None));
    }

    let flags = PruneFlags { all: true, dry_run: true, project: None };
    let result =
        workspace_prune_inner(&ctx.state, &ctx.event_bus, &flags, &workspaces_dir, &cancel).await;

    match result {
        Ok(Response::WorkspacesPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 1, "should report 1 workspace");
            assert_eq!(skipped, 0);
        }
        other => panic!("expected WorkspacesPruned, got: {:?}", other),
    }

    assert!(ws_path.exists(), "workspace dir should remain after dry run");

    let s = ctx.state.lock();
    assert!(s.workspaces.contains_key("ws-keep"), "workspace should remain in state after dry run");
}

#[tokio::test]
async fn workspace_prune_includes_orphaned_owner_workspaces_with_namespace() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());
    let cancel = CancellationToken::new();

    let workspaces_dir = dir.path().join("workspaces");
    std::fs::create_dir_all(&workspaces_dir).unwrap();

    {
        let mut s = ctx.state.lock();
        s.workspaces.insert(
            "ws-orphan-owner".to_string(),
            make_workspace(
                "ws-orphan-owner",
                workspaces_dir.join("ws-orphan-owner"),
                Some("deleted-job-id"),
            ),
        );
        s.jobs.insert("live-job".to_string(), make_job_ns("live-job", "done", "myproject"));
        s.workspaces.insert(
            "ws-with-owner".to_string(),
            make_workspace("ws-with-owner", workspaces_dir.join("ws-with-owner"), Some("live-job")),
        );
    }

    let flags = PruneFlags { all: true, dry_run: false, project: Some("myproject") };
    let result =
        workspace_prune_inner(&ctx.state, &ctx.event_bus, &flags, &workspaces_dir, &cancel).await;

    match result {
        Ok(Response::WorkspacesPruned { pruned, .. }) => {
            let ids: Vec<&str> = pruned.iter().map(|ws| ws.id.as_str()).collect();
            assert!(
                ids.contains(&"ws-orphan-owner"),
                "orphaned owner workspace should be pruned, got: {:?}",
                ids
            );
            assert!(
                ids.contains(&"ws-with-owner"),
                "matching project workspace should be pruned, got: {:?}",
                ids
            );
        }
        other => panic!("expected WorkspacesPruned, got: {:?}", other),
    }
}

#[tokio::test]
async fn workspace_prune_returns_cancelled_when_token_is_cancelled() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());
    let cancel = CancellationToken::new();

    let workspaces_dir = dir.path().join("workspaces");
    std::fs::create_dir_all(&workspaces_dir).unwrap();

    let ws_path = workspaces_dir.join("ws-cancel-test");
    std::fs::create_dir_all(&ws_path).unwrap();

    {
        let mut s = ctx.state.lock();
        s.workspaces.insert(
            "ws-cancel-test".to_string(),
            make_workspace("ws-cancel-test", ws_path.clone(), None),
        );
    }

    // Cancel before the handler runs
    cancel.cancel();

    let flags = PruneFlags { all: true, dry_run: false, project: None };
    let result =
        workspace_prune_inner(&ctx.state, &ctx.event_bus, &flags, &workspaces_dir, &cancel).await;

    match result {
        Ok(Response::Error { message }) => {
            assert!(
                message.contains("cancelled"),
                "expected cancellation message, got: {}",
                message
            );
        }
        other => panic!("expected Error with cancelled message, got: {:?}", other),
    }

    // Directory should still exist since we cancelled before deletion
    assert!(ws_path.exists(), "workspace dir should remain after cancellation");
}
