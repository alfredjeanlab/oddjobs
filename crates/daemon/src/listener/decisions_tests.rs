// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::{
    build_question_resume_message, build_resume_message, map_decision_to_agent_run_action,
    map_decision_to_job_action, resolve_decision_action, ResolvedAction,
};
use oj_core::{AgentRunId, AgentRunStatus, DecisionOption, DecisionSource, Event};

#[test]
fn idle_dismiss_returns_no_action() {
    let result = map_decision_to_job_action(
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
    let result = map_decision_to_job_action(
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
    let result = map_decision_to_job_action(
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
    let result = map_decision_to_job_action(
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

// ===================== Tests for agent run action mapping =====================

#[test]
fn agent_run_idle_nudge_emits_resume() {
    let ar_id = AgentRunId::new("ar-123");
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Idle,
        Some(1), // Nudge
        Some("please continue"),
        "dec-ar1",
        &ar_id,
        Some("session-abc"),
        &[],
    );

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentRunResume { id, message, kill } => {
            assert_eq!(id.as_str(), "ar-123");
            assert_eq!(message.as_deref(), Some("please continue"));
            assert!(!kill);
        }
        other => panic!("expected AgentRunResume, got {:?}", other),
    }
}

#[test]
fn agent_run_idle_done_marks_completed() {
    let ar_id = AgentRunId::new("ar-456");
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Idle,
        Some(2), // Done
        None,
        "dec-ar2",
        &ar_id,
        Some("session-xyz"),
        &[],
    );

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentRunStatusChanged { id, status, .. } => {
            assert_eq!(id.as_str(), "ar-456");
            assert_eq!(*status, AgentRunStatus::Completed);
        }
        other => panic!("expected AgentRunStatusChanged, got {:?}", other),
    }
}

#[test]
fn agent_run_idle_cancel_marks_failed() {
    let ar_id = AgentRunId::new("ar-789");
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Idle,
        Some(3), // Cancel
        None,
        "dec-ar3",
        &ar_id,
        Some("session-123"),
        &[],
    );

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentRunStatusChanged { id, status, reason } => {
            assert_eq!(id.as_str(), "ar-789");
            assert_eq!(*status, AgentRunStatus::Failed);
            assert!(reason.as_ref().unwrap().contains("cancelled"));
        }
        other => panic!("expected AgentRunStatusChanged(Failed), got {:?}", other),
    }
}

#[test]
fn agent_run_idle_dismiss_returns_empty() {
    let ar_id = AgentRunId::new("ar-000");
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Idle,
        Some(4), // Dismiss
        None,
        "dec-ar4",
        &ar_id,
        Some("session-456"),
        &[],
    );

    assert!(events.is_empty());
}

#[test]
fn agent_run_error_retry_emits_resume_with_kill() {
    let ar_id = AgentRunId::new("ar-err1");
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Error,
        Some(1), // Retry
        None,
        "dec-err1",
        &ar_id,
        Some("session-err"),
        &[],
    );

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentRunResume { id, message, kill } => {
            assert_eq!(id.as_str(), "ar-err1");
            assert!(message.is_some());
            assert!(*kill);
        }
        other => panic!("expected AgentRunResume(kill=true), got {:?}", other),
    }
}

#[test]
fn agent_run_error_skip_marks_completed() {
    let ar_id = AgentRunId::new("ar-err2");
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Error,
        Some(2), // Skip
        None,
        "dec-err2",
        &ar_id,
        Some("session-err2"),
        &[],
    );

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentRunStatusChanged { id, status, .. } => {
            assert_eq!(id.as_str(), "ar-err2");
            assert_eq!(*status, AgentRunStatus::Completed);
        }
        other => panic!("expected AgentRunStatusChanged(Completed), got {:?}", other),
    }
}

#[test]
fn agent_run_approval_approve_sends_y() {
    let ar_id = AgentRunId::new("ar-approve");
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Approval,
        Some(1), // Approve
        None,
        "dec-approve",
        &ar_id,
        Some("session-approve"),
        &[],
    );

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-approve");
            assert_eq!(input, "y\n");
        }
        other => panic!("expected SessionInput(y), got {:?}", other),
    }
}

#[test]
fn agent_run_approval_deny_sends_n() {
    let ar_id = AgentRunId::new("ar-deny");
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Approval,
        Some(2), // Deny
        None,
        "dec-deny",
        &ar_id,
        Some("session-deny"),
        &[],
    );

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-deny");
            assert_eq!(input, "n\n");
        }
        other => panic!("expected SessionInput(n), got {:?}", other),
    }
}

#[test]
fn agent_run_question_sends_option_number() {
    let ar_id = AgentRunId::new("ar-q1");
    let options = make_question_options();
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Question,
        Some(2), // Option B
        None,
        "dec-q1",
        &ar_id,
        Some("session-q1"),
        &options,
    );

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-q1");
            assert_eq!(input, "2\n");
        }
        other => panic!("expected SessionInput(2), got {:?}", other),
    }
}

#[test]
fn agent_run_question_cancel_marks_failed() {
    let ar_id = AgentRunId::new("ar-qcancel");
    let options = make_question_options(); // 3 options, Cancel is last
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Question,
        Some(3), // Cancel (last option)
        None,
        "dec-qcancel",
        &ar_id,
        Some("session-qcancel"),
        &options,
    );

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentRunStatusChanged { id, status, .. } => {
            assert_eq!(id.as_str(), "ar-qcancel");
            assert_eq!(*status, AgentRunStatus::Failed);
        }
        other => panic!("expected AgentRunStatusChanged(Failed), got {:?}", other),
    }
}

#[test]
fn agent_run_no_session_nudge_still_emits_resume() {
    let ar_id = AgentRunId::new("ar-nosession");
    let events = map_decision_to_agent_run_action(
        &DecisionSource::Idle,
        Some(1), // Nudge
        Some("continue"),
        "dec-nosession",
        &ar_id,
        None, // No session — AgentRunResume handles liveness check in engine
        &[],
    );

    // AgentRunResume is emitted regardless of session; the engine handles liveness
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentRunResume { id, message, kill } => {
            assert_eq!(id.as_str(), "ar-nosession");
            assert_eq!(message.as_deref(), Some("continue"));
            assert!(!kill);
        }
        other => panic!("expected AgentRunResume, got {:?}", other),
    }
}

// ===================== Tests for resolve_decision_action =====================

#[test]
fn resolve_no_choice_returns_freeform() {
    assert_eq!(
        resolve_decision_action(&DecisionSource::Idle, None, &[]),
        ResolvedAction::Freeform
    );
}

#[test]
fn resolve_idle_choices() {
    let opts = &[];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Idle, Some(1), opts),
        ResolvedAction::Nudge
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Idle, Some(2), opts),
        ResolvedAction::Complete
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Idle, Some(3), opts),
        ResolvedAction::Cancel
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Idle, Some(4), opts),
        ResolvedAction::Dismiss
    );
}

#[test]
fn resolve_error_choices() {
    let opts = &[];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Error, Some(1), opts),
        ResolvedAction::Retry
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Error, Some(2), opts),
        ResolvedAction::Complete
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Error, Some(3), opts),
        ResolvedAction::Cancel
    );
}

#[test]
fn resolve_gate_choices() {
    let opts = &[];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Gate, Some(1), opts),
        ResolvedAction::Retry
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Gate, Some(2), opts),
        ResolvedAction::Complete
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Gate, Some(3), opts),
        ResolvedAction::Cancel
    );
}

#[test]
fn resolve_approval_choices() {
    let opts = &[];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Approval, Some(1), opts),
        ResolvedAction::Approve
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Approval, Some(2), opts),
        ResolvedAction::Deny
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Approval, Some(3), opts),
        ResolvedAction::Cancel
    );
}

#[test]
fn resolve_question_cancel_is_last_option() {
    let options = make_question_options(); // 3 options
    assert_eq!(
        resolve_decision_action(&DecisionSource::Question, Some(3), &options),
        ResolvedAction::Cancel
    );
}

#[test]
fn resolve_question_non_cancel_is_answer() {
    let options = make_question_options(); // 3 options
    assert_eq!(
        resolve_decision_action(&DecisionSource::Question, Some(1), &options),
        ResolvedAction::Answer
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Question, Some(2), &options),
        ResolvedAction::Answer
    );
}

#[test]
fn resolve_question_option_3_is_not_cancel_when_more_options() {
    // 4 user options + Cancel = 5 total; option 3 should be Answer, not Cancel
    let options = vec![
        DecisionOption {
            label: "A".to_string(),
            description: None,
            recommended: false,
        },
        DecisionOption {
            label: "B".to_string(),
            description: None,
            recommended: false,
        },
        DecisionOption {
            label: "C".to_string(),
            description: None,
            recommended: false,
        },
        DecisionOption {
            label: "D".to_string(),
            description: None,
            recommended: false,
        },
        DecisionOption {
            label: "Cancel".to_string(),
            description: None,
            recommended: false,
        },
    ];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Question, Some(3), &options),
        ResolvedAction::Answer,
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Question, Some(5), &options),
        ResolvedAction::Cancel,
    );
}
