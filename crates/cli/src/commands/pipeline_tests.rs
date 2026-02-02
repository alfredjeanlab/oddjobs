// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::{parse_duration, status_group};
use oj_daemon::PipelineSummary;
use std::time::Duration;

#[test]
fn parse_duration_seconds() {
    assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
}

#[test]
fn parse_duration_minutes() {
    assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
}

#[test]
fn parse_duration_hours() {
    assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
}

#[test]
fn parse_duration_combined() {
    assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
}

#[test]
fn parse_duration_bare_number() {
    assert_eq!(parse_duration("60").unwrap(), Duration::from_secs(60));
}

#[test]
fn parse_duration_zero_fails() {
    assert!(parse_duration("0s").is_err());
}

#[test]
fn status_group_running_is_active() {
    assert_eq!(status_group("build", "Running"), 0);
}

#[test]
fn status_group_pending_is_active() {
    assert_eq!(status_group("execute", "Pending"), 0);
}

#[test]
fn status_group_waiting_is_active() {
    assert_eq!(status_group("review", "Waiting"), 0);
}

#[test]
fn status_group_failed_step() {
    assert_eq!(status_group("failed", "Failed"), 1);
}

#[test]
fn status_group_failed_step_any_status() {
    assert_eq!(status_group("failed", "Running"), 1);
}

#[test]
fn status_group_done_step() {
    assert_eq!(status_group("done", "Completed"), 2);
}

#[test]
fn status_group_step_status_failed() {
    assert_eq!(status_group("build", "Failed"), 1);
}

#[test]
fn status_group_step_status_completed() {
    assert_eq!(status_group("build", "Completed"), 2);
}

#[test]
fn sort_order_active_before_failed_before_done() {
    let mut pipelines = vec![
        PipelineSummary {
            id: "done-1".into(),
            name: "done-pipeline".into(),
            kind: "build".into(),
            step: "done".into(),
            step_status: "Completed".into(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
            namespace: String::new(),
        },
        PipelineSummary {
            id: "failed-1".into(),
            name: "failed-pipeline".into(),
            kind: "build".into(),
            step: "failed".into(),
            step_status: "Failed".into(),
            created_at_ms: 2000,
            updated_at_ms: 2000,
            namespace: String::new(),
        },
        PipelineSummary {
            id: "active-1".into(),
            name: "active-pipeline".into(),
            kind: "build".into(),
            step: "build".into(),
            step_status: "Running".into(),
            created_at_ms: 3000,
            updated_at_ms: 3000,
            namespace: String::new(),
        },
    ];

    pipelines.sort_by(|a, b| {
        let ga = status_group(&a.step, &a.step_status);
        let gb = status_group(&b.step, &b.step_status);
        ga.cmp(&gb).then(b.created_at_ms.cmp(&a.created_at_ms))
    });

    assert_eq!(pipelines[0].id, "active-1");
    assert_eq!(pipelines[1].id, "failed-1");
    assert_eq!(pipelines[2].id, "done-1");
}

#[test]
fn sort_order_most_recent_first_within_group() {
    let mut pipelines = vec![
        PipelineSummary {
            id: "old".into(),
            name: "old-pipeline".into(),
            kind: "build".into(),
            step: "build".into(),
            step_status: "Running".into(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
            namespace: String::new(),
        },
        PipelineSummary {
            id: "new".into(),
            name: "new-pipeline".into(),
            kind: "build".into(),
            step: "execute".into(),
            step_status: "Running".into(),
            created_at_ms: 5000,
            updated_at_ms: 5000,
            namespace: String::new(),
        },
    ];

    pipelines.sort_by(|a, b| {
        let ga = status_group(&a.step, &a.step_status);
        let gb = status_group(&b.step, &b.step_status);
        ga.cmp(&gb).then(b.created_at_ms.cmp(&a.created_at_ms))
    });

    assert_eq!(pipelines[0].id, "new");
    assert_eq!(pipelines[1].id, "old");
}
