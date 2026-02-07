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
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"));
    assert!(result.is_none());
}

#[test]
fn build_resume_message_with_choice() {
    let c = DecisionResolveCtx {
        chosen: Some(2),
        ..ctx(&DecisionSource::Idle, "dec-123")
    };
    let msg = build_resume_message(&c);
    assert!(msg.contains("option 2"));
    assert!(msg.contains("dec-123"));
}

#[test]
fn build_resume_message_with_message_only() {
    let c = DecisionResolveCtx {
        message: Some("looks good"),
        ..ctx(&DecisionSource::Idle, "dec-123")
    };
    let msg = build_resume_message(&c);
    assert!(msg.contains("looks good"));
    assert!(msg.contains("dec-123"));
}

#[test]
fn build_resume_message_with_both() {
    let c = DecisionResolveCtx {
        chosen: Some(1),
        message: Some("approved"),
        ..ctx(&DecisionSource::Idle, "dec-123")
    };
    let msg = build_resume_message(&c);
    assert!(msg.contains("option 1"));
    assert!(msg.contains("approved"));
}

fn make_question_options() -> Vec<DecisionOption> {
    vec![
        DecisionOption::new("Option A").description("First option"),
        DecisionOption::new("Option B").description("Second option"),
        DecisionOption::new("Cancel").description("Cancel the job"),
    ]
}

#[test]
fn question_cancel_is_last_option() {
    let options = make_question_options();
    // Cancel is option 3 (last)
    let c = DecisionResolveCtx {
        chosen: Some(3),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"));
    match result {
        Some(Event::JobCancel { .. }) => {}
        other => panic!("expected JobCancel, got {:?}", other),
    }
}

#[test]
fn question_non_cancel_choice_resumes_with_label() {
    let options = make_question_options();
    let c = DecisionResolveCtx {
        chosen: Some(1),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"));
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
    let options = make_question_options();
    let c = DecisionResolveCtx {
        message: Some("custom answer"),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"));
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

#[test]
fn question_resume_message_no_choice_no_message() {
    let options = make_question_options();
    let c = DecisionResolveCtx {
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let msg = build_question_resume_message(&c);
    assert!(msg.contains("dec-q1"), "expected decision id, got: {}", msg);
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
fn agent_run_question_cancel_marks_failed() {
    let ar_id = AgentRunId::new("ar-qcancel");
    let options = make_question_options(); // 3 options, Cancel is last
    let c = DecisionResolveCtx {
        chosen: Some(3), // Cancel (last option)
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
        DecisionOption::new("A"),
        DecisionOption::new("B"),
        DecisionOption::new("C"),
        DecisionOption::new("D"),
        DecisionOption::new("Cancel"),
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
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"));

    match result {
        Some(Event::JobResume { message, .. }) => {
            let msg = message.unwrap();
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
    assert!(msg.contains("dec-1"));
    assert!(msg.contains("1, 2"));
}

// ===================== Tests for Plan decision resolution =====================

#[test]
fn resolve_plan_accept_clear_context() {
    let opts = &[];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Plan, Some(1), opts),
        ResolvedAction::Approve
    );
}

#[test]
fn resolve_plan_accept_auto_edits() {
    let opts = &[];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Plan, Some(2), opts),
        ResolvedAction::Approve
    );
}

#[test]
fn resolve_plan_accept_manual_edits() {
    let opts = &[];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Plan, Some(3), opts),
        ResolvedAction::Approve
    );
}

#[test]
fn resolve_plan_revise() {
    let opts = &[];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Plan, Some(4), opts),
        ResolvedAction::Freeform
    );
}

#[test]
fn resolve_plan_cancel() {
    let opts = &[];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Plan, Some(5), opts),
        ResolvedAction::Cancel
    );
}

#[test]
fn resolve_plan_freeform_no_choice() {
    let opts = &[];
    assert_eq!(
        resolve_decision_action(&DecisionSource::Plan, None, opts),
        ResolvedAction::Freeform
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
fn plan_job_approve_emits_resume() {
    let c = DecisionResolveCtx {
        chosen: Some(1), // Accept
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"));

    match result {
        Some(Event::JobResume { message, .. }) => {
            let msg = message.unwrap();
            assert!(msg.contains("plan approved"), "got: {}", msg);
        }
        other => panic!("expected JobResume, got {:?}", other),
    }
}

#[test]
fn plan_job_revise_emits_resume_with_revision() {
    let c = DecisionResolveCtx {
        chosen: Some(4), // Revise
        message: Some("Add error handling"),
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"));

    match result {
        Some(Event::JobResume { message, .. }) => {
            let msg = message.unwrap();
            assert!(msg.contains("plan revision"), "got: {}", msg);
            assert!(msg.contains("Add error handling"), "got: {}", msg);
        }
        other => panic!("expected JobResume, got {:?}", other),
    }
}

#[test]
fn plan_job_cancel_emits_cancel() {
    let c = DecisionResolveCtx {
        chosen: Some(5), // Cancel
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "pipe-1", Some("step-1"));

    match result {
        Some(Event::JobCancel { .. }) => {}
        other => panic!("expected JobCancel, got {:?}", other),
    }
}
