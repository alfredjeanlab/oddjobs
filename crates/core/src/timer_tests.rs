// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::pipeline::PipelineId;

#[test]
fn timer_id_display() {
    let id = TimerId::new("test-timer");
    assert_eq!(id.to_string(), "test-timer");
}

#[test]
fn timer_id_equality() {
    let id1 = TimerId::new("timer-1");
    let id2 = TimerId::new("timer-1");
    let id3 = TimerId::new("timer-2");

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
}

#[test]
fn timer_id_from_str() {
    let id: TimerId = "test".into();
    assert_eq!(id.as_str(), "test");
}

#[test]
fn timer_id_serde() {
    let id = TimerId::new("my-timer");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"my-timer\"");

    let parsed: TimerId = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, id);
}

#[test]
fn liveness_timer_id() {
    let pipeline_id = PipelineId::new("pipe-123");
    let id = TimerId::liveness(&pipeline_id);
    assert_eq!(id.as_str(), "liveness:pipe-123");
}

#[test]
fn exit_deferred_timer_id() {
    let pipeline_id = PipelineId::new("pipe-123");
    let id = TimerId::exit_deferred(&pipeline_id);
    assert_eq!(id.as_str(), "exit-deferred:pipe-123");
}

#[test]
fn cooldown_timer_id_format() {
    let pipeline_id = PipelineId::new("pipe-123");
    let id = TimerId::cooldown(&pipeline_id, "idle", 0);
    assert_eq!(id.as_str(), "cooldown:pipe-123:idle:0");

    let pipeline_id2 = PipelineId::new("pipe-456");
    let id2 = TimerId::cooldown(&pipeline_id2, "exit", 2);
    assert_eq!(id2.as_str(), "cooldown:pipe-456:exit:2");
}

#[test]
fn is_liveness() {
    let id = TimerId::new("liveness:pipe-123");
    assert!(id.is_liveness());

    let id = TimerId::new("exit-deferred:pipe-123");
    assert!(!id.is_liveness());

    let id = TimerId::new("cooldown:pipe-123:idle:0");
    assert!(!id.is_liveness());
}

#[test]
fn is_exit_deferred() {
    let id = TimerId::new("exit-deferred:pipe-123");
    assert!(id.is_exit_deferred());

    let id = TimerId::new("liveness:pipe-123");
    assert!(!id.is_exit_deferred());

    let id = TimerId::new("cooldown:pipe-123:idle:0");
    assert!(!id.is_exit_deferred());
}

#[test]
fn is_cooldown() {
    let id = TimerId::new("cooldown:pipe-123:idle:0");
    assert!(id.is_cooldown());

    let id = TimerId::new("liveness:pipe-123");
    assert!(!id.is_cooldown());

    let id = TimerId::new("exit-deferred:pipe-123");
    assert!(!id.is_cooldown());
}

#[test]
fn pipeline_id_str_liveness() {
    let id = TimerId::new("liveness:pipe-123");
    assert_eq!(id.pipeline_id_str(), Some("pipe-123"));
}

#[test]
fn pipeline_id_str_exit_deferred() {
    let id = TimerId::new("exit-deferred:pipe-456");
    assert_eq!(id.pipeline_id_str(), Some("pipe-456"));
}

#[test]
fn pipeline_id_str_cooldown() {
    let id = TimerId::new("cooldown:pipe-789:idle:0");
    assert_eq!(id.pipeline_id_str(), Some("pipe-789"));
}

#[test]
fn pipeline_id_str_unknown_timer() {
    let id = TimerId::new("other-timer");
    assert_eq!(id.pipeline_id_str(), None);
}

#[test]
fn queue_retry_timer_id_format() {
    let id = TimerId::queue_retry("bugs", "item-123");
    assert_eq!(id.as_str(), "queue-retry:bugs:item-123");
}

#[test]
fn queue_retry_timer_id_with_namespace() {
    let id = TimerId::queue_retry("myns/bugs", "item-456");
    assert_eq!(id.as_str(), "queue-retry:myns/bugs:item-456");
}

#[test]
fn is_queue_retry() {
    let id = TimerId::queue_retry("bugs", "item-1");
    assert!(id.is_queue_retry());

    let id = TimerId::new("liveness:pipe-123");
    assert!(!id.is_queue_retry());

    let id = TimerId::new("cooldown:pipe-123:idle:0");
    assert!(!id.is_queue_retry());
}
