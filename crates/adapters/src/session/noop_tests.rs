// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use std::path::Path;

#[tokio::test]
async fn noop_session_spawn_returns_noop_id() {
    let adapter = NoOpSessionAdapter::new();
    let id = adapter
        .spawn("test", Path::new("/tmp"), "cmd", &[], &[])
        .await
        .unwrap();
    assert_eq!(id, "noop");
}

#[tokio::test]
async fn noop_session_send_returns_ok() {
    let adapter = NoOpSessionAdapter::new();
    let result = adapter.send("id", "input").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn noop_session_kill_returns_ok() {
    let adapter = NoOpSessionAdapter::new();
    let result = adapter.kill("id").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn noop_session_is_alive_returns_false() {
    let adapter = NoOpSessionAdapter::new();
    let alive = adapter.is_alive("id").await.unwrap();
    assert!(!alive);
}

#[tokio::test]
async fn noop_session_capture_output_returns_empty() {
    let adapter = NoOpSessionAdapter::new();
    let output = adapter.capture_output("id", 100).await.unwrap();
    assert!(output.is_empty());
}

#[tokio::test]
async fn noop_session_is_process_running_returns_false() {
    let adapter = NoOpSessionAdapter::new();
    let running = adapter.is_process_running("id", "pattern").await.unwrap();
    assert!(!running);
}

#[test]
fn noop_session_default() {
    let adapter = NoOpSessionAdapter::default();
    assert!(std::mem::size_of_val(&adapter) == 0);
}
