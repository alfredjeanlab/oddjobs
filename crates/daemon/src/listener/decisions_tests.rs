// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::{
    build_multi_question_resume_message, build_question_resume_message, build_resume_message,
    map_decision_to_crew_action, map_decision_to_job_action, resolve_decision_action,
    DecisionResolveCtx, ResolvedAction,
};
use oj_core::{CrewId, CrewStatus, DecisionOption, DecisionSource, Event};

/// Build a `DecisionResolveCtx` with sensible defaults for tests.
/// Override fields as needed after construction.
fn ctx<'a>(source: &'a DecisionSource, decision_id: &'a str) -> DecisionResolveCtx<'a> {
    DecisionResolveCtx {
        source,
        choices: &[],
        message: None,
        decision_id,
        options: &[],
        questions: None,
    }
}

#[test]
fn idle_dismiss_returns_no_action() {
    let c = DecisionResolveCtx { choices: &[4], ..ctx(&DecisionSource::Idle, "dec-123") };
    let result = map_decision_to_job_action(&c, "job-1", Some("step-1"), None);
    assert!(result.is_empty());
}

#[yare::parameterized(
    choice_only  = { &[2],  None,              "Please continue with the task." },
    message_only = { &[],   Some("looks good"), "looks good" },
    both         = { &[1],  Some("approved"),   "approved" },
)]
fn build_resume_msg(choices: &[usize], message: Option<&str>, expected: &str) {
    let c = DecisionResolveCtx { choices, message, ..ctx(&DecisionSource::Idle, "dec-123") };
    assert_eq!(build_resume_message(&c), expected);
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

#[yare::parameterized(
    other_freeform = { &[3],  Some("install from git repo"), &["install from git repo"] as &[&str] },
    choice_label   = { &[1],  None,                          &["Option A", "option 1"] as &[&str] },
    freeform_only  = { &[],   Some("custom answer"),         &["custom answer"] as &[&str] },
)]
fn question_job_resume(choices: &[usize], message: Option<&str>, expected_contains: &[&str]) {
    let options = make_question_options();
    let c = DecisionResolveCtx {
        choices,
        message,
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "job-1", Some("step-1"), None);
    assert_eq!(result.len(), 1);
    match &result[0] {
        Event::JobResume { message, .. } => {
            let msg = message.as_ref().unwrap();
            for expected in expected_contains {
                assert!(msg.contains(expected), "expected '{}' in message, got: {}", expected, msg);
            }
        }
        other => panic!("expected JobResume, got {:?}", other),
    }
}

#[test]
fn question_cancel_is_second_to_last_option() {
    let options = make_question_options();
    // Cancel is option 4 (second-to-last, before Dismiss)
    let c = DecisionResolveCtx {
        choices: &[4],
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "job-1", Some("step-1"), None);
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
        choices: &[5],
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let result = map_decision_to_job_action(&c, "job-1", Some("step-1"), None);
    assert!(result.is_empty(), "Dismiss should produce no action events");
}

#[test]
fn question_choice_with_message() {
    let options = make_question_options();
    let c = DecisionResolveCtx {
        choices: &[2],
        message: Some("extra context"),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let msg = build_question_resume_message(&c);
    assert!(msg.contains("Option B"), "expected label, got: {}", msg);
    assert!(msg.contains("extra context"), "expected message, got: {}", msg);
}

// ===================== Tests for crew action mapping =====================

#[derive(Debug)]
enum ExpectedAgentAction {
    Resume { kill: bool },
    Status(CrewStatus),
    Empty,
}

#[yare::parameterized(
    idle_nudge   = { DecisionSource::Idle,  &[1], Some("please continue"), ExpectedAgentAction::Resume { kill: false } },
    idle_done    = { DecisionSource::Idle,  &[2], None,                    ExpectedAgentAction::Status(CrewStatus::Completed) },
    idle_cancel  = { DecisionSource::Idle,  &[3], None,                    ExpectedAgentAction::Status(CrewStatus::Failed) },
    idle_dismiss = { DecisionSource::Idle,  &[4], None,                    ExpectedAgentAction::Empty },
    error_retry  = { DecisionSource::Error, &[1], None,                    ExpectedAgentAction::Resume { kill: true } },
    error_skip   = { DecisionSource::Error, &[2], None,                    ExpectedAgentAction::Status(CrewStatus::Completed) },
    dead_dismiss = { DecisionSource::Dead,  &[4], None,                    ExpectedAgentAction::Empty },
)]
fn crew_action(
    source: DecisionSource,
    choices: &[usize],
    message: Option<&str>,
    expected: ExpectedAgentAction,
) {
    let run_id = CrewId::from_string("run-test");
    let c = DecisionResolveCtx { choices, message, ..ctx(&source, "dec-test") };
    let events = map_decision_to_crew_action(&c, &run_id, Some("agent-test"));

    match expected {
        ExpectedAgentAction::Resume { kill } => {
            assert_eq!(events.len(), 1);
            match &events[0] {
                Event::CrewResume { id, message: m, kill: k } => {
                    assert_eq!(id.as_str(), "run-test");
                    assert_eq!(*k, kill);
                    if let Some(msg) = message {
                        assert_eq!(m.as_deref(), Some(msg));
                    } else {
                        assert!(m.is_some(), "expected auto-generated message");
                    }
                }
                other => panic!("expected CrewResume, got {:?}", other),
            }
        }
        ExpectedAgentAction::Status(status) => {
            assert_eq!(events.len(), 1);
            match &events[0] {
                Event::CrewUpdated { id, status: s, .. } => {
                    assert_eq!(id.as_str(), "run-test");
                    assert_eq!(*s, status);
                }
                other => panic!("expected CrewUpdated, got {:?}", other),
            }
        }
        ExpectedAgentAction::Empty => {
            assert!(events.is_empty());
        }
    }
}

#[yare::parameterized(
    approve = { &[1], "y\n" },
    deny    = { &[2], "n\n" },
)]
fn crew_approval(choices: &[usize], expected_input: &str) {
    let run_id = CrewId::from_string("run-approval");
    let c = DecisionResolveCtx { choices, ..ctx(&DecisionSource::Approval, "dec-approval") };
    let events = map_decision_to_crew_action(&c, &run_id, Some("agent-approval"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentInput { id, input } => {
            assert_eq!(id.as_str(), "agent-approval");
            assert_eq!(input, expected_input);
        }
        other => panic!("expected AgentInput({}), got {:?}", expected_input, other),
    }
}

#[test]
fn crew_question_sends_option_number() {
    let run_id = CrewId::from_string("run-q1");
    let options = make_question_options();
    let c = DecisionResolveCtx {
        choices: &[2], // Option B
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-q1")
    };
    let events = map_decision_to_crew_action(&c, &run_id, Some("agent-q1"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentInput { id: agent_id, input } => {
            assert_eq!(agent_id.as_str(), "agent-q1");
            assert_eq!(input, "2\n");
        }
        other => panic!("expected AgentInput(2), got {:?}", other),
    }
}

#[test]
fn crew_question_other_sends_custom_text() {
    let run_id = CrewId::from_string("run-qother");
    let options = make_question_options(); // 5 options, Other is third-to-last
    let c = DecisionResolveCtx {
        choices: &[3], // Other (third-to-last option)
        message: Some("install from git repo"),
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-qother")
    };
    let events = map_decision_to_crew_action(&c, &run_id, Some("agent-qother"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentInput { id: agent_id, input } => {
            assert_eq!(agent_id.as_str(), "agent-qother");
            assert_eq!(input, "install from git repo\n");
        }
        other => panic!("expected AgentInput, got {:?}", other),
    }
}

#[test]
fn crew_question_cancel_marks_failed() {
    let run_id = CrewId::from_string("run-qcancel");
    let options = make_question_options(); // 5 options, Cancel is second-to-last
    let c = DecisionResolveCtx {
        choices: &[4], // Cancel (second-to-last option)
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-qcancel")
    };
    let events = map_decision_to_crew_action(&c, &run_id, Some("agent-qcancel"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::CrewUpdated { id, status, .. } => {
            assert_eq!(id.as_str(), "run-qcancel");
            assert_eq!(*status, CrewStatus::Failed);
        }
        other => panic!("expected CrewUpdated(Failed), got {:?}", other),
    }
}

#[test]
fn crew_question_dismiss_returns_empty() {
    let run_id = CrewId::from_string("run-qdismiss");
    let options = make_question_options(); // 5 options, Dismiss is last
    let c = DecisionResolveCtx {
        choices: &[5], // Dismiss (last option)
        options: &options,
        ..ctx(&DecisionSource::Question, "dec-qdismiss")
    };
    let events = map_decision_to_crew_action(&c, &run_id, Some("agent-qdismiss"));
    assert!(events.is_empty());
}

#[test]
fn crew_no_session_nudge_still_emits_resume() {
    let run_id = CrewId::from_string("run-nosession");
    let c = DecisionResolveCtx {
        choices: &[1], // Nudge
        message: Some("continue"),
        ..ctx(&DecisionSource::Idle, "dec-nosession")
    };
    // No session — CrewResume handles liveness check in engine
    let events = map_decision_to_crew_action(&c, &run_id, None);

    // CrewResume is emitted regardless of session; the engine handles liveness
    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::CrewResume { id, message, kill } => {
            assert_eq!(id.as_str(), "run-nosession");
            assert_eq!(message.as_deref(), Some("continue"));
            assert!(!kill);
        }
        other => panic!("expected CrewResume, got {:?}", other),
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
    let c = DecisionResolveCtx { choices: &[4], ..ctx(&DecisionSource::Error, "dec-err-dismiss") };
    let result = map_decision_to_job_action(&c, "job-1", Some("step-1"), None);
    assert!(result.is_empty());
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
    assert_eq!(resolve_decision_action(&DecisionSource::Question, choice, &options), expected);
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

fn make_multi_questions() -> QuestionData {
    QuestionData {
        questions: vec![
            QuestionEntry {
                question: "Which framework?".to_string(),
                header: Some("Framework".to_string()),
                options: vec![
                    QuestionOption { label: "React".to_string(), description: None },
                    QuestionOption { label: "Vue".to_string(), description: None },
                ],
                multi_select: false,
            },
            QuestionEntry {
                question: "Which database?".to_string(),
                header: Some("Database".to_string()),
                options: vec![
                    QuestionOption { label: "PostgreSQL".to_string(), description: None },
                    QuestionOption { label: "MySQL".to_string(), description: None },
                ],
                multi_select: false,
            },
        ],
    }
}

#[test]
fn multi_question_crew_sends_concatenated_digits() {
    let run_id = CrewId::from_string("run-multi");
    let qd = make_multi_questions();
    let choices = [1, 2]; // Q1: React, Q2: MySQL
    let c = DecisionResolveCtx {
        choices: &choices,
        questions: Some(&qd),
        ..ctx(&DecisionSource::Question, "dec-multi")
    };
    let events = map_decision_to_crew_action(&c, &run_id, Some("agent-multi"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentInput { id: agent_id, input } => {
            assert_eq!(agent_id.as_str(), "agent-multi");
            // Concatenated digits: "12\n"
            assert_eq!(input, "12\n");
        }
        other => panic!("expected AgentInput, got {:?}", other),
    }
}

#[test]
fn multi_question_job_sends_resume_with_labels() {
    let qd = make_multi_questions();
    let choices = [1, 2];
    let c = DecisionResolveCtx {
        choices: &choices,
        questions: Some(&qd),
        ..ctx(&DecisionSource::Question, "dec-multi")
    };
    let result = map_decision_to_job_action(&c, "job-1", Some("step-1"), None);

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
    let qd = make_multi_questions();
    let choices = [1, 2];
    let c = DecisionResolveCtx {
        choices: &choices,
        questions: Some(&qd),
        ..ctx(&DecisionSource::Question, "dec-1")
    };
    let msg = build_multi_question_resume_message(&c);
    assert!(msg.contains("Framework: React (1)"));
    assert!(msg.contains("Database: MySQL (2)"));
}

#[test]
fn build_multi_question_resume_message_without_data() {
    let choices = [1, 2];
    let c = DecisionResolveCtx { choices: &choices, ..ctx(&DecisionSource::Question, "dec-1") };
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
    assert_eq!(resolve_decision_action(&DecisionSource::Plan, choice, &[]), expected);
}

#[yare::parameterized(
    option1 = { &[1], 1 },
    option2 = { &[2], 2 },
    option3 = { &[3], 3 },
)]
fn crew_plan_accept(choices: &[usize], expected_option: u32) {
    let run_id = CrewId::from_string("run-plan");
    let c = DecisionResolveCtx { choices, ..ctx(&DecisionSource::Plan, "dec-plan") };
    let events = map_decision_to_crew_action(&c, &run_id, Some("agent-plan"));

    assert_eq!(events.len(), 1);
    match &events[0] {
        Event::AgentRespond { id, response } => {
            assert_eq!(id.as_str(), "agent-plan");
            assert_eq!(response.option, Some(expected_option));
        }
        other => panic!("expected AgentRespond with option {}, got {:?}", expected_option, other),
    }
}

#[test]
fn crew_plan_revise_sends_respond_then_resume() {
    let run_id = CrewId::from_string("run-plan-rev");
    let c = DecisionResolveCtx {
        choices: &[4], // Revise
        message: Some("Please also add rate limiting"),
        ..ctx(&DecisionSource::Plan, "dec-plan-rev")
    };
    let events = map_decision_to_crew_action(&c, &run_id, Some("agent-plan"));

    assert_eq!(events.len(), 2);
    // First: AgentRespond with revision text
    match &events[0] {
        Event::AgentRespond { id: agent_id, response } => {
            assert_eq!(agent_id.as_str(), "agent-plan");
            assert_eq!(response.text.as_deref(), Some("Please also add rate limiting"));
        }
        other => panic!("expected AgentRespond with text, got {:?}", other),
    }
    // Second: CrewResume with revision feedback
    match &events[1] {
        Event::CrewResume { id, message, kill } => {
            assert_eq!(id.as_str(), "run-plan-rev");
            assert_eq!(message.as_deref(), Some("Please also add rate limiting"));
            assert!(!kill);
        }
        other => panic!("expected CrewResume, got {:?}", other),
    }
}

#[test]
fn crew_plan_cancel_sends_reject_then_fail() {
    let run_id = CrewId::from_string("run-plan-cancel");
    let c = DecisionResolveCtx {
        choices: &[5], // Cancel
        ..ctx(&DecisionSource::Plan, "dec-plan-cancel")
    };
    let events = map_decision_to_crew_action(&c, &run_id, Some("agent-plan"));

    assert_eq!(events.len(), 2);
    // First: AgentRespond with accept=false to reject the plan
    match &events[0] {
        Event::AgentRespond { id: agent_id, response } => {
            assert_eq!(agent_id.as_str(), "agent-plan");
            assert_eq!(response.accept, Some(false));
        }
        other => panic!("expected AgentRespond with accept=false, got {:?}", other),
    }
    // Second: Fail the crew
    match &events[1] {
        Event::CrewUpdated { id, status, reason } => {
            assert_eq!(id.as_str(), "run-plan-cancel");
            assert_eq!(*status, CrewStatus::Failed);
            assert!(reason.as_ref().unwrap().contains("plan rejected"));
        }
        other => panic!("expected CrewUpdated(Failed), got {:?}", other),
    }
}

#[yare::parameterized(
    option1 = { &[1], 1 },
    option2 = { &[2], 2 },
    option3 = { &[3], 3 },
)]
fn plan_job_accept(choices: &[usize], expected_option: u32) {
    let c = DecisionResolveCtx { choices, ..ctx(&DecisionSource::Plan, "dec-plan-job") };
    let result = map_decision_to_job_action(&c, "job-1", Some("step-1"), Some("agent-plan"));

    assert_eq!(result.len(), 2);
    assert!(
        matches!(&result[0], Event::StepStarted { job_id, step, .. } if job_id.as_str() == "job-1" && step == "step-1"),
        "expected StepStarted, got {:?}",
        result[0]
    );
    match &result[1] {
        Event::AgentRespond { id, response } => {
            assert_eq!(id.as_str(), "agent-plan");
            assert_eq!(response.option, Some(expected_option));
        }
        other => panic!("expected AgentRespond with option {}, got {:?}", expected_option, other),
    }
}

#[test]
fn plan_job_accept_no_session_emits_nothing() {
    let c = DecisionResolveCtx {
        choices: &[1], // Accept
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "job-1", Some("step-1"), None);

    // Without an agent, we can't send key presses — but StepStarted still emitted
    assert_eq!(result.len(), 1);
    assert!(matches!(&result[0], Event::StepStarted { .. }));
}

#[test]
fn plan_job_revise_sends_respond_then_resume() {
    let c = DecisionResolveCtx {
        choices: &[4], // Revise
        message: Some("Add error handling"),
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "job-1", Some("step-1"), Some("agent-plan"));

    assert_eq!(result.len(), 2);
    // First: AgentRespond with revision text
    match &result[0] {
        Event::AgentRespond { id: agent_id, response } => {
            assert_eq!(agent_id.as_str(), "agent-plan");
            assert_eq!(response.text.as_deref(), Some("Add error handling"));
        }
        other => panic!("expected AgentRespond with text, got {:?}", other),
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
fn plan_job_cancel_sends_reject_then_cancel() {
    let c = DecisionResolveCtx {
        choices: &[5], // Cancel
        ..ctx(&DecisionSource::Plan, "dec-plan-job")
    };
    let result = map_decision_to_job_action(&c, "job-1", Some("step-1"), Some("agent-plan"));

    assert_eq!(result.len(), 2);
    // First: AgentRespond with accept=false to reject the plan
    match &result[0] {
        Event::AgentRespond { id: agent_id, response } => {
            assert_eq!(agent_id.as_str(), "agent-plan");
            assert_eq!(response.accept, Some(false));
        }
        other => panic!("expected AgentRespond with accept=false, got {:?}", other),
    }
    // Second: JobCancel
    match &result[1] {
        Event::JobCancel { .. } => {}
        other => panic!("expected JobCancel, got {:?}", other),
    }
}
