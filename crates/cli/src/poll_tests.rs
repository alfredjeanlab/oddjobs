// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::time::Duration;

use super::*;

#[tokio::test]
async fn tick_returns_ready_before_deadline() {
    let mut poller = Poller::new(Duration::from_millis(10), Some(Duration::from_secs(5)));
    let result = poller.tick().await;
    assert!(matches!(result, Tick::Ready));
}

#[tokio::test]
async fn tick_returns_timeout_when_deadline_expires_during_sleep() {
    let mut poller = Poller::new(Duration::from_millis(50), Some(Duration::from_millis(1)));
    let result = poller.tick().await;
    assert!(matches!(result, Tick::Timeout));
}

#[tokio::test]
async fn tick_returns_timeout_when_already_expired() {
    let mut poller = Poller::new(Duration::from_millis(10), Some(Duration::ZERO));
    // Deadline is already in the past
    let result = poller.tick().await;
    assert!(matches!(result, Tick::Timeout));
}

#[tokio::test]
async fn tick_ready_multiple_times() {
    let mut poller = Poller::new(Duration::from_millis(10), Some(Duration::from_secs(5)));
    for _ in 0..3 {
        let result = poller.tick().await;
        assert!(matches!(result, Tick::Ready));
    }
}

#[tokio::test]
async fn tick_no_timeout_polls_indefinitely() {
    let mut poller = Poller::new(Duration::from_millis(10), None);
    for _ in 0..5 {
        let result = poller.tick().await;
        assert!(matches!(result, Tick::Ready));
    }
}
