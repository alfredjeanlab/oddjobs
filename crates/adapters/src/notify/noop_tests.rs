// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[tokio::test]
async fn noop_notify_returns_ok() {
    let adapter = NoOpNotifyAdapter::new();
    let result = adapter.notify("title", "message").await;
    assert!(result.is_ok());
}

#[test]
fn noop_notify_default() {
    let adapter = NoOpNotifyAdapter::default();
    assert!(std::mem::size_of_val(&adapter) == 0);
}
