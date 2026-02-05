// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::{build_question_resume_message, build_resume_message, map_decision_to_action};
use oj_core::{DecisionOption, DecisionSource};

#[test]
fn idle_dismiss_returns_no_action() {
    let result = map_decision_to_action(
        &DecisionSource::Idle,
        Some(4),
        None,
        "dec-123",
        "pipe-1",
        Some("step-1"),
        &[],
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

fn make_question_options() -> Vec<DecisionOption> {
    vec![
        DecisionOption {
            label: "Option A".to_string(),
            description: Some("First option".to_string()),
            recommended: false,
        },
        DecisionOption {
            label: "Option B".to_string(),
            description: Some("Second option".to_string()),
            recommended: false,
        },
        DecisionOption {
            label: "Cancel".to_string(),
            description: Some("Cancel the job".to_string()),
            recommended: false,
        },
    ]
}

#[test]
fn question_cancel_is_last_option() {
    use oj_core::Event;
    let options = make_question_options();
    // Cancel is option 3 (last)
    let result = map_decision_to_action(
        &DecisionSource::Question,
        Some(3),
        None,
        "dec-q1",
        "pipe-1",
        Some("step-1"),
        &options,
    );
    match result {
        Some(Event::JobCancel { .. }) => {}
        other => panic!("expected JobCancel, got {:?}", other),
    }
}

#[test]
fn question_non_cancel_choice_resumes_with_label() {
    use oj_core::Event;
    let options = make_question_options();
    let result = map_decision_to_action(
        &DecisionSource::Question,
        Some(1),
        None,
        "dec-q1",
        "pipe-1",
        Some("step-1"),
        &options,
    );
    match result {
        Some(Event::JobResume { message, .. }) => {
            let msg = message.unwrap();
            assert!(msg.contains("Option A"), "expected label, got: {}", msg);
            assert!(
                msg.contains("option 1"),
                "expected option number, got: {}",
                msg
            );
        }
        other => panic!("expected JobResume, got {:?}", other),
    }
}

#[test]
fn question_freeform_message_only() {
    use oj_core::Event;
    let options = make_question_options();
    let result = map_decision_to_action(
        &DecisionSource::Question,
        None,
        Some("custom answer"),
        "dec-q1",
        "pipe-1",
        Some("step-1"),
        &options,
    );
    match result {
        Some(Event::JobResume { message, .. }) => {
            let msg = message.unwrap();
            assert!(
                msg.contains("custom answer"),
                "expected freeform message, got: {}",
                msg
            );
        }
        other => panic!("expected JobResume, got {:?}", other),
    }
}

#[test]
fn question_choice_with_message() {
    let options = make_question_options();
    let msg = build_question_resume_message(Some(2), Some("extra context"), "dec-q1", &options);
    assert!(msg.contains("Option B"), "expected label, got: {}", msg);
    assert!(
        msg.contains("extra context"),
        "expected message, got: {}",
        msg
    );
}

#[test]
fn question_resume_message_no_choice_no_message() {
    let options = make_question_options();
    let msg = build_question_resume_message(None, None, "dec-q1", &options);
    assert!(msg.contains("dec-q1"), "expected decision id, got: {}", msg);
}
