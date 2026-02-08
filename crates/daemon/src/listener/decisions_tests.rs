// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::{
    build_multi_question_resume_message, build_question_resume_message, build_resume_message,
    map_decision_to_agent_run_action, map_decision_to_job_action, resolve_decision_action,
    DecisionResolveCtx, ResolvedAction,
};
use oj_core::{AgentRunId, AgentRunStatus, DecisionOption, DecisionSource, Event};

/// Build a `DecisionResolveCtx` with sensible defaults for tests.
/// Override fields as needed after construction.
fn ctx<'a>(source: &'a DecisionSource, decision_id: &'a str) -> DecisionResolveCtx<'a> {
    DecisionResolveCtx {
        source,
        chosen: None,
        choices: &[],
        message: None,
        decision_id,
        options: &[],
        question_data: None,
    }
}

#[test]
fn idle_dismiss_returns_no_action() {
    let c = DecisionResolveCtx {
        chosen: Some(4),
        ..ctx(&DecisionSource::Idle, "dec-123")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), None);
    assert!(result.is_empty());
}

#[test]
fn build_resume_message_with_choice_only() {
    let c = DecisionResolveCtx {
        chosen: Some(2),
        ..ctx(&DecisionSource::Idle, "dec-123")
    };
    let msg = build_resume_message(&c);
    assert_eq!(msg, "Please continue with the task.");
}

#[test]
fn build_resume_message_with_message_only() {
    let c = DecisionResolveCtx {
        message: Some("looks good"),
        ..ctx(&DecisionSource::Idle, "dec-123")
    };
    let msg = build_resume_message(&c);
    assert_eq!(msg, "looks good");
}

#[test]
fn build_resume_message_with_both() {
    let c = DecisionResolveCtx {
        chosen: Some(1),
        message: Some("approved"),
        ..ctx(&DecisionSource::Idle, "dec-123")
    };
    let msg = build_resume_message(&c);
    assert_eq!(msg, "approved");
}

fn make_question_options() -> Vec<DecisionOption> {
    vec![
        DecisionOption::new("Option A").description("First option"),
        DecisionOption::new("Option B").description("Second option"),
        DecisionOption::new("Other").description("Write a custom response"),
        DecisionOption::new("Cancel").description("Cancel the job"),
        DecisionOption::new("Dismiss").description("Dismiss this notification"),
    ]
}

#[test]
fn question_other_sends_freeform_resume() {
    let options = make_question_options();
    // Other is option 3 (third-to-last)
    let c = DecisionResolveCtx {
        chosen: Some(3),
        message: Some("install from git repo"),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), None);
    assert_eq!(result.len(), 1);
    match &result[0] {
        Event::JobResume { message, .. } => {
            let msg = message.as_ref().unwrap();
            assert!(
                msg.contains("install from git repo"),
                "expected freeform message, got: {}",
                msg
            );
        }
        other => panic!("expected JobResume, got {:?}", other),
    }
}

#[test]
fn question_cancel_is_second_to_last_option() {
    let options = make_question_options();
    // Cancel is option 4 (second-to-last, before Dismiss)
    let c = DecisionResolveCtx {
        chosen: Some(4),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), None);
    assert_eq!(result.len(), 1);
    match &result[0] {
        Event::JobCancel { .. } => {}
        other => panic!("expected JobCancel, got {:?}", other),
    }
}

#[test]
fn question_dismiss_is_last_option() {
    let options = make_question_options();
    // Dismiss is option 5 (last)
    let c = DecisionResolveCtx {
        chosen: Some(5),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), None);
    assert!(result.is_empty(), "Dismiss should produce no action events");
}

#[test]
fn question_non_cancel_choice_resumes_with_label() {
    let options = make_question_options();
    let c = DecisionResolveCtx {
        chosen: Some(1),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), None);
    assert_eq!(result.len(), 1);
    match &result[0] {
        Event::JobResume { message, .. } => {
            let msg = message.as_ref().unwrap();
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
    let options = make_question_options();
    let c = DecisionResolveCtx {
        message: Some("custom answer"),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), None);
    assert_eq!(result.len(), 1);
    match &result[0] {
        Event::JobResume { message, .. } => {
            let msg = message.as_ref().unwrap();
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
    let c = DecisionResolveCtx {
        chosen: Some(2),
        message: Some("extra context"),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let msg = build_question_resume_message(&c);
    assert!(msg.contains("Option B"), "expected label, got: {}", msg);
    assert!(
        msg.contains("extra context"),
        "expected message, got: {}",
        msg
    );
}

// ===================== Tests for agent run action mapping =====================

#[test]
fn agent_run_idle_nudge_emits_resume() {
    let ar_id = AgentRunId::new("ar-123");
    let c = DecisionResolveCtx {
        chosen: Some(1), // Nudge
        message: Some("please continue"),
        ..ctx(&DecisionSource::Idle, "dec-ar1")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-abc"));

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
    let c = DecisionResolveCtx {
        chosen: Some(2), // Done
        ..ctx(&DecisionSource::Idle, "dec-ar2")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-xyz"));

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
    let c = DecisionResolveCtx {
        chosen: Some(3), // Cancel
        ..ctx(&DecisionSource::Idle, "dec-ar3")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-123"));

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
    let c = DecisionResolveCtx {
        chosen: Some(4), // Dismiss
        ..ctx(&DecisionSource::Idle, "dec-ar4")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-456"));

    assert!(events.is_empty());
}

#[test]
fn agent_run_error_retry_emits_resume_with_kill() {
    let ar_id = AgentRunId::new("ar-err1");
    let c = DecisionResolveCtx {
        chosen: Some(1), // Retry
        ..ctx(&DecisionSource::Error, "dec-err1")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-err"));

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
    let c = DecisionResolveCtx {
        chosen: Some(2), // Skip
        ..ctx(&DecisionSource::Error, "dec-err2")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-err2"));

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
    let c = DecisionResolveCtx {
        chosen: Some(1), // Approve
        ..ctx(&DecisionSource::Approval, "dec-approve")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-approve"));

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
    let c = DecisionResolveCtx {
        chosen: Some(2), // Deny
        ..ctx(&DecisionSource::Approval, "dec-deny")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-deny"));

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
    let c = DecisionResolveCtx {
        chosen: Some(2), // Option B
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-q1"));

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
fn agent_run_question_other_sends_custom_text() {
    let ar_id = AgentRunId::new("ar-qother");
    let options = make_question_options(); // 5 options, Other is third-to-last
    let c = DecisionResolveCtx {
        chosen: Some(3), // Other (third-to-last option)
        message: Some("install from git repo"),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-qother")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-qother"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-qother");
            assert_eq!(input, "install from git repo\n");
        }
        other => panic!("expected SessionInput, got {:?}", other),
    }
}

#[test]
fn agent_run_question_cancel_marks_failed() {
    let ar_id = AgentRunId::new("ar-qcancel");
    let options = make_question_options(); // 5 options, Cancel is second-to-last
    let c = DecisionResolveCtx {
        chosen: Some(4), // Cancel (second-to-last option)
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-qcancel")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-qcancel"));

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
fn agent_run_question_dismiss_returns_empty() {
    let ar_id = AgentRunId::new("ar-qdismiss");
    let options = make_question_options(); // 5 options, Dismiss is last
    let c = DecisionResolveCtx {
        chosen: Some(5), // Dismiss (last option)
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-qdismiss")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-qdismiss"));
    assert!(events.is_empty());
}

#[test]
fn agent_run_no_session_nudge_still_emits_resume() {
    let ar_id = AgentRunId::new("ar-nosession");
    let c = DecisionResolveCtx {
        chosen: Some(1), // Nudge
        message: Some("continue"),
        ..ctx(&DecisionSource::Idle, "dec-nosession")
    };
    // No session — AgentRunResume handles liveness check in engine
    let events = map_decision_to_agent_run_action(&c, &ar_id, None);

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

#[yare::parameterized(
    no_choice_freeform = { DecisionSource::Idle,  None,    ResolvedAction::Freeform },
    idle_nudge         = { DecisionSource::Idle,  Some(1), ResolvedAction::Nudge },
    idle_complete      = { DecisionSource::Idle,  Some(2), ResolvedAction::Complete },
    idle_cancel        = { DecisionSource::Idle,  Some(3), ResolvedAction::Cancel },
    idle_dismiss       = { DecisionSource::Idle,  Some(4), ResolvedAction::Dismiss },
    error_retry        = { DecisionSource::Error, Some(1), ResolvedAction::Retry },
    error_complete     = { DecisionSource::Error, Some(2), ResolvedAction::Complete },
    error_cancel       = { DecisionSource::Error, Some(3), ResolvedAction::Cancel },
    error_dismiss      = { DecisionSource::Error, Some(4), ResolvedAction::Dismiss },
    dead_retry         = { DecisionSource::Dead,  Some(1), ResolvedAction::Retry },
    dead_complete      = { DecisionSource::Dead,  Some(2), ResolvedAction::Complete },
    dead_cancel        = { DecisionSource::Dead,  Some(3), ResolvedAction::Cancel },
    dead_dismiss       = { DecisionSource::Dead,  Some(4), ResolvedAction::Dismiss },
)]
fn resolve_fixed_source(source: DecisionSource, choice: Option<usize>, expected: ResolvedAction) {
    assert_eq!(resolve_decision_action(&source, choice, &[]), expected);
}

#[test]
fn error_dismiss_returns_no_action() {
    let c = DecisionResolveCtx {
        chosen: Some(4),
        ..ctx(&DecisionSource::Error, "dec-err-dismiss")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), None);
    assert!(result.is_empty());
}

#[test]
fn agent_run_dead_dismiss_returns_empty() {
    let ar_id = AgentRunId::new("ar-dead-dismiss");
    let c = DecisionResolveCtx {
        chosen: Some(4), // Dismiss
        ..ctx(&DecisionSource::Dead, "dec-dead-dismiss")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-dead"));
    assert!(events.is_empty());
}

#[yare::parameterized(
    gate_retry         = { DecisionSource::Gate,     Some(1), ResolvedAction::Retry },
    gate_complete      = { DecisionSource::Gate,     Some(2), ResolvedAction::Complete },
    gate_cancel        = { DecisionSource::Gate,     Some(3), ResolvedAction::Cancel },
    approval_approve   = { DecisionSource::Approval, Some(1), ResolvedAction::Approve },
    approval_deny      = { DecisionSource::Approval, Some(2), ResolvedAction::Deny },
    approval_cancel    = { DecisionSource::Approval, Some(3), ResolvedAction::Cancel },
    approval_dismiss   = { DecisionSource::Approval, Some(4), ResolvedAction::Dismiss },
)]
fn resolve_gate_and_approval(
    source: DecisionSource,
    choice: Option<usize>,
    expected: ResolvedAction,
) {
    assert_eq!(resolve_decision_action(&source, choice, &[]), expected);
}

// Question option positions: 1..N=Answer, N+1=Other(Freeform), N+2=Cancel, N+3=Dismiss
#[yare::parameterized(
    answer_1         = { Some(1), ResolvedAction::Answer },
    answer_2         = { Some(2), ResolvedAction::Answer },
    other            = { Some(3), ResolvedAction::Freeform },
    cancel           = { Some(4), ResolvedAction::Cancel },
    dismiss          = { Some(5), ResolvedAction::Dismiss },
    freeform         = { None,    ResolvedAction::Freeform },
)]
fn resolve_question_choices(choice: Option<usize>, expected: ResolvedAction) {
    // 5 options: A, B, Other, Cancel, Dismiss
    let options = make_question_options();
    assert_eq!(
        resolve_decision_action(&DecisionSource::Question, choice, &options),
        expected
    );
}

#[test]
fn resolve_question_dynamic_positions_with_more_options() {
    // 4 user options + Other + Cancel + Dismiss = 7 total
    let options = vec![
        DecisionOption::new("A"),
        DecisionOption::new("B"),
        DecisionOption::new("C"),
        DecisionOption::new("D"),
        DecisionOption::new("Other"),
        DecisionOption::new("Cancel"),
        DecisionOption::new("Dismiss"),
    ];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Question, Some(3), &options),
        ResolvedAction::Answer
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Question, Some(5), &options),
        ResolvedAction::Freeform
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Question, Some(6), &options),
        ResolvedAction::Cancel
    );
    assert_eq!(
        resolve_decision_action(&DecisionSource::Question, Some(7), &options),
        ResolvedAction::Dismiss
    );
}

// ===================== Tests for multi-question routing =====================

use oj_core::{QuestionData, QuestionEntry, QuestionOption};

fn make_multi_question_data() -> QuestionData {
    QuestionData {
        questions: vec![
            QuestionEntry {
                question: "Which framework?".to_string(),
                header: Some("Framework".to_string()),
                options: vec![
                    QuestionOption {
                        label: "React".to_string(),
                        description: None,
                    },
                    QuestionOption {
                        label: "Vue".to_string(),
                        description: None,
                    },
                ],
                multi_select: false,
            },
            QuestionEntry {
                question: "Which database?".to_string(),
                header: Some("Database".to_string()),
                options: vec![
                    QuestionOption {
                        label: "PostgreSQL".to_string(),
                        description: None,
                    },
                    QuestionOption {
                        label: "MySQL".to_string(),
                        description: None,
                    },
                ],
                multi_select: false,
            },
        ],
    }
}

#[test]
fn multi_question_agent_run_sends_concatenated_digits() {
    let ar_id = AgentRunId::new("ar-multi");
    let qd = make_multi_question_data();
    let choices = [1, 2]; // Q1: React, Q2: MySQL
    let c = DecisionResolveCtx {
        choices: &choices,
        question_data: Some(&qd),
        ..ctx(&DecisionSource::Question, "dec-multi")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-multi"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-multi");
            // Concatenated digits: "12\n"
            assert_eq!(input, "12\n");
        }
        other => panic!("expected SessionInput, got {:?}", other),
    }
}

#[test]
fn multi_question_job_sends_resume_with_labels() {
    let qd = make_multi_question_data();
    let choices = [1, 2];
    let c = DecisionResolveCtx {
        choices: &choices,
        question_data: Some(&qd),
        ..ctx(&DecisionSource::Question, "dec-multi")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), None);

    assert_eq!(result.len(), 1);
    match &result[0] {
        Event::JobResume { message, .. } => {
            let msg = message.as_ref().unwrap();
            assert!(msg.contains("React"), "expected React label, got: {}", msg);
            assert!(msg.contains("MySQL"), "expected MySQL label, got: {}", msg);
        }
        other => panic!("expected JobResume, got {:?}", other),
    }
}

#[test]
fn build_multi_question_resume_message_with_data() {
    let qd = make_multi_question_data();
    let choices = [1, 2];
    let c = DecisionResolveCtx {
        choices: &choices,
        question_data: Some(&qd),
        ..ctx(&DecisionSource::Question, "dec-1")
    };
    let msg = build_multi_question_resume_message(&c);
    assert!(msg.contains("Framework: React (1)"));
    assert!(msg.contains("Database: MySQL (2)"));
}

#[test]
fn build_multi_question_resume_message_without_data() {
    let choices = [1, 2];
    let c = DecisionResolveCtx {
        choices: &choices,
        ..ctx(&DecisionSource::Question, "dec-1")
    };
    let msg = build_multi_question_resume_message(&c);
    assert_eq!(msg, "Selected: choices [1, 2]");
}

// ===================== Tests for Plan decision resolution =====================

#[yare::parameterized(
    accept_clear_context = { Some(1), ResolvedAction::Approve },
    accept_auto_edits    = { Some(2), ResolvedAction::Approve },
    accept_manual_edits  = { Some(3), ResolvedAction::Approve },
    revise               = { Some(4), ResolvedAction::Freeform },
    cancel               = { Some(5), ResolvedAction::Cancel },
    freeform_no_choice   = { None,    ResolvedAction::Freeform },
)]
fn resolve_plan_choices(choice: Option<usize>, expected: ResolvedAction) {
    assert_eq!(
        resolve_decision_action(&DecisionSource::Plan, choice, &[]),
        expected
    );
}

#[test]
fn agent_run_plan_accept_sends_enter() {
    let ar_id = AgentRunId::new("ar-plan1");
    let c = DecisionResolveCtx {
        chosen: Some(1), // Accept (clear context) — already selected, just Enter
        ..ctx(&DecisionSource::Plan, "dec-plan1")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-plan"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-plan");
            // Option 1 is already selected, so just "Enter"
            assert_eq!(input, "Enter");
        }
        other => panic!("expected SessionInput(Enter), got {:?}", other),
    }
}

#[test]
fn agent_run_plan_accept_option2_sends_down_enter() {
    let ar_id = AgentRunId::new("ar-plan2");
    let c = DecisionResolveCtx {
        chosen: Some(2), // Accept (auto edits) — 1 Down + Enter
        ..ctx(&DecisionSource::Plan, "dec-plan2")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-plan"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-plan");
            assert_eq!(input, "Down Enter");
        }
        other => panic!("expected SessionInput(Down Enter), got {:?}", other),
    }
}

#[test]
fn agent_run_plan_accept_option3_sends_down_down_enter() {
    let ar_id = AgentRunId::new("ar-plan3");
    let c = DecisionResolveCtx {
        chosen: Some(3), // Accept (manual edits) — 2 Downs + Enter
        ..ctx(&DecisionSource::Plan, "dec-plan3")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-plan"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-plan");
            assert_eq!(input, "Down Down Enter");
        }
        other => panic!("expected SessionInput(Down Down Enter), got {:?}", other),
    }
}

#[test]
fn agent_run_plan_revise_sends_escape_then_resume() {
    let ar_id = AgentRunId::new("ar-plan-rev");
    let c = DecisionResolveCtx {
        chosen: Some(4), // Revise
        message: Some("Please also add rate limiting"),
        ..ctx(&DecisionSource::Plan, "dec-plan-rev")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-plan"));

    assert_eq!(events.len(), 2);
    // First: Escape to cancel the plan dialog
    match &events[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-plan");
            assert_eq!(input, "Escape");
        }
        other => panic!("expected SessionInput(Escape), got {:?}", other),
    }
    // Second: AgentRunResume with revision feedback
    match &events[1] {
        Event::AgentRunResume { id, message, kill } => {
            assert_eq!(id.as_str(), "ar-plan-rev");
            assert_eq!(message.as_deref(), Some("Please also add rate limiting"));
            assert!(!kill);
        }
        other => panic!("expected AgentRunResume, got {:?}", other),
    }
}

#[test]
fn agent_run_plan_cancel_sends_escape_then_fail() {
    let ar_id = AgentRunId::new("ar-plan-cancel");
    let c = DecisionResolveCtx {
        chosen: Some(5), // Cancel
        ..ctx(&DecisionSource::Plan, "dec-plan-cancel")
    };
    let events = map_decision_to_agent_run_action(&c, &ar_id, Some("session-plan"));

    assert_eq!(events.len(), 2);
    // First: Escape to cancel the plan dialog
    match &events[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-plan");
            assert_eq!(input, "Escape");
        }
        other => panic!("expected SessionInput(Escape), got {:?}", other),
    }
    // Second: Fail the agent run
    match &events[1] {
        Event::AgentRunStatusChanged { id, status, reason } => {
            assert_eq!(id.as_str(), "ar-plan-cancel");
            assert_eq!(*status, AgentRunStatus::Failed);
            assert!(reason.as_ref().unwrap().contains("plan rejected"));
        }
        other => panic!("expected AgentRunStatusChanged(Failed), got {:?}", other),
    }
}

#[test]
fn plan_job_accept_sends_enter() {
    let c = DecisionResolveCtx {
        chosen: Some(1), // Accept (clear context) — already selected, just Enter
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), Some("session-plan"));

    assert_eq!(result.len(), 1);
    match &result[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-plan");
            assert_eq!(input, "Enter");
        }
        other => panic!("expected SessionInput(Enter), got {:?}", other),
    }
}

#[test]
fn plan_job_accept_option2_sends_down_enter() {
    let c = DecisionResolveCtx {
        chosen: Some(2), // Accept (auto edits) — 1 Down + Enter
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), Some("session-plan"));

    assert_eq!(result.len(), 1);
    match &result[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-plan");
            assert_eq!(input, "Down Enter");
        }
        other => panic!("expected SessionInput(Down Enter), got {:?}", other),
    }
}

#[test]
fn plan_job_accept_option3_sends_down_down_enter() {
    let c = DecisionResolveCtx {
        chosen: Some(3), // Accept (manual edits) — 2 Downs + Enter
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), Some("session-plan"));

    assert_eq!(result.len(), 1);
    match &result[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-plan");
            assert_eq!(input, "Down Down Enter");
        }
        other => panic!("expected SessionInput(Down Down Enter), got {:?}", other),
    }
}

#[test]
fn plan_job_accept_no_session_emits_nothing() {
    let c = DecisionResolveCtx {
        chosen: Some(1), // Accept
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), None);

    // Without a session, we can't send key presses
    assert!(result.is_empty());
}

#[test]
fn plan_job_revise_sends_escape_then_resume() {
    let c = DecisionResolveCtx {
        chosen: Some(4), // Revise
        message: Some("Add error handling"),
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), Some("session-plan"));

    assert_eq!(result.len(), 2);
    // First: Escape to cancel the plan dialog
    match &result[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-plan");
            assert_eq!(input, "Escape");
        }
        other => panic!("expected SessionInput(Escape), got {:?}", other),
    }
    // Second: JobResume with revision feedback (user message only, no decision ID)
    match &result[1] {
        Event::JobResume { message, .. } => {
            assert_eq!(message.as_deref(), Some("Add error handling"));
        }
        other => panic!("expected JobResume, got {:?}", other),
    }
}

#[test]
fn plan_job_cancel_sends_escape_then_cancel() {
    let c = DecisionResolveCtx {
        chosen: Some(5), // Cancel
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"), Some("session-plan"));

    assert_eq!(result.len(), 2);
    // First: Escape to cancel the plan dialog
    match &result[0] {
        Event::SessionInput { id, input } => {
            assert_eq!(id.as_str(), "session-plan");
            assert_eq!(input, "Escape");
        }
        other => panic!("expected SessionInput(Escape), got {:?}", other),
    }
    // Second: JobCancel
    match &result[1] {
        Event::JobCancel { .. } => {}
        other => panic!("expected JobCancel, got {:?}", other),
    }
}
