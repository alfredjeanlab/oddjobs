// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use tempfile::tempdir;

use crate::protocol::Response;

use super::super::test_helpers::test_ctx;
use super::handle_session_kill;

#[tokio::test]
async fn session_kill_nonexistent_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let result = handle_session_kill(&ctx, "nonexistent-session").await;

    match result {
        Ok(Response::Error { message }) => {
            assert!(
                message.contains("not found"),
                "expected 'not found' in message, got: {}",
                message
            );
        }
        other => panic!("expected Response::Error, got: {:?}", other),
    }
}

#[tokio::test]
async fn session_kill_existing_returns_ok() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        s.sessions.insert(
            "oj-test-session".to_string(),
            oj_storage::Session {
                id: "oj-test-session".to_string(),
                job_id: "pipe-1".to_string(),
            },
        );
    }

    let result = handle_session_kill(&ctx, "oj-test-session").await;
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}
