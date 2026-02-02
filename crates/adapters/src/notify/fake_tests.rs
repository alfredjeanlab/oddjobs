// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[tokio::test]
async fn fake_notify_records_calls() {
    let adapter = FakeNotifyAdapter::new();

    adapter.send("alerts", "Pipeline started").await.unwrap();
    adapter.send("alerts", "Pipeline completed").await.unwrap();

    let calls = adapter.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].channel, "alerts");
    assert_eq!(calls[0].message, "Pipeline started");
}
