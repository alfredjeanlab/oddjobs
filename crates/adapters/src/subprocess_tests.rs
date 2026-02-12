// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use tokio::process::Command;

#[tokio::test]
async fn run_with_timeout_success() {
    let mut cmd = Command::new("echo");
    cmd.arg("hello");
    let output = run_with_timeout(cmd, Duration::from_secs(5), "echo").await.unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
}

#[tokio::test]
async fn run_with_timeout_nonzero_exit_is_not_an_error() {
    let cmd = Command::new("false");
    let output = run_with_timeout(cmd, Duration::from_secs(5), "false").await.unwrap();
    assert!(!output.status.success());
}

#[tokio::test]
async fn run_with_timeout_io_error() {
    let cmd = Command::new("/nonexistent/binary");
    let result = run_with_timeout(cmd, Duration::from_secs(5), "nonexistent").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.starts_with("nonexistent failed:"), "got: {}", err);
}

#[tokio::test]
async fn run_with_timeout_timeout_elapsed() {
    let mut cmd = Command::new("sleep");
    cmd.arg("10");
    let result = run_with_timeout(cmd, Duration::from_millis(100), "test sleep").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("timed out"), "got: {}", err);
    assert!(err.contains("test sleep"), "got: {}", err);
}
