// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::{build_resume_message, map_decision_to_action};
use oj_core::DecisionSource;

#[test]
fn idle_dismiss_returns_no_action() {
    let result = map_decision_to_action(
        &DecisionSource::Idle,
        Some(4),
        None,
        "dec-123",
        "pipe-1",
        Some("step-1"),
    );
    assert!(result.is_none());
}

#[test]
fn build_resume_message_with_choice() {
    let msg = build_resume_message(Some(2), None, "dec-123");
    assert!(msg.contains("option 2"));
    assert!(msg.contains("dec-123"));
}

#[test]
fn build_resume_message_with_message_only() {
    let msg = build_resume_message(None, Some("looks good"), "dec-123");
    assert!(msg.contains("looks good"));
    assert!(msg.contains("dec-123"));
}

#[test]
fn build_resume_message_with_both() {
    let msg = build_resume_message(Some(1), Some("approved"), "dec-123");
    assert!(msg.contains("option 1"));
    assert!(msg.contains("approved"));
}
