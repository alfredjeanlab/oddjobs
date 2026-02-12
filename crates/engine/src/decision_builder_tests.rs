// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::{
    AgentId, CrewId, DecisionId, DecisionOption, DecisionSource, Event, JobId, OwnerId,
    QuestionData, QuestionEntry, QuestionOption,
};
use proptest::prelude::*;

/// Unwrap a `DecisionCreated` event, panicking if the variant doesn't match.
struct Decision {
    id: DecisionId,
    agent_id: AgentId,
    owner: OwnerId,
    source: DecisionSource,
    context: String,
    options: Vec<DecisionOption>,
    questions: Option<QuestionData>,
    project: String,
}

fn unwrap_decision((id, event): (DecisionId, Event)) -> Decision {
    match event {
        Event::DecisionCreated {
            id: did,
            agent_id,
            owner,
            source,
            context,
            options,
            questions,
            project,
            ..
        } => {
            assert_eq!(id, did);
            Decision { id, agent_id, owner, source, context, options, questions, project }
        }
        _ => panic!("expected DecisionCreated"),
    }
}

fn build_job_decision(trigger: EscalationTrigger) -> Decision {
    unwrap_decision(
        EscalationDecisionBuilder::new(
            JobId::new("job-1").into(),
            "test-job".to_string(),
            "agent-1".to_string(),
            trigger,
        )
        .build(),
    )
}

#[yare::parameterized(
    idle   = {
        EscalationTrigger::Idle { last_message: None },
        DecisionSource::Idle,
        &["Nudge", "Done", "Cancel", "Dismiss"],
        true
    },
    dead   = {
        EscalationTrigger::Dead { exit_code: Some(137), last_message: None },
        DecisionSource::Dead,
        &["Retry", "Skip", "Cancel", "Dismiss"],
        true
    },
    error  = {
        EscalationTrigger::Error { error_type: "OutOfCredits".into(), message: "API quota exceeded".into(), last_message: None },
        DecisionSource::Error,
        &["Retry", "Skip", "Cancel", "Dismiss"],
        true
    },
    prompt = {
        EscalationTrigger::Prompt { prompt_type: "permission".into(), last_message: None },
        DecisionSource::Approval,
        &["Approve", "Deny", "Cancel", "Dismiss"],
        false
    },
    plan   = {
        EscalationTrigger::Plan { last_message: Some("plan".into()) },
        DecisionSource::Plan,
        &["Accept (clear context)", "Accept (auto edits)", "Accept (manual edits)", "Revise", "Cancel"],
        true
    },
)]
fn trigger_builds_correct_options(
    trigger: EscalationTrigger,
    expected_source: DecisionSource,
    expected_labels: &[&str],
    first_recommended: bool,
) {
    let d = build_job_decision(trigger);
    assert!(!d.id.as_str().is_empty());
    assert_eq!(d.source, expected_source);
    assert_eq!(d.options.len(), expected_labels.len());
    for (opt, label) in d.options.iter().zip(expected_labels) {
        assert_eq!(opt.label, *label);
    }
    assert_eq!(
        d.options[0].recommended, first_recommended,
        "first option recommended mismatch for {:?}",
        d.options[0].label,
    );
}

#[yare::parameterized(
    gate_with_stderr = {
        EscalationTrigger::GateFailed { command: "./check.sh".into(), exit_code: 1, stderr: "validation failed".into() },
        &["./check.sh", "validation failed", "Exit code: 1"],
        &[]
    },
    gate_empty_stderr = {
        EscalationTrigger::GateFailed { command: "./check.sh".into(), exit_code: 1, stderr: String::new() },
        &["./check.sh", "Exit code: 1"],
        &["stderr:"]
    },
    prompt_type = {
        EscalationTrigger::Prompt { prompt_type: "permission".into(), last_message: None },
        &["permission prompt"],
        &[]
    },
    dead_no_exit_code = {
        EscalationTrigger::Dead { exit_code: None, last_message: None },
        &["exited unexpectedly"],
        &["exit code"]
    },
    plan_with_context = {
        EscalationTrigger::Plan { last_message: Some("# My Plan\n\n## Steps\n1. Do thing".into()) },
        &["requesting plan approval", "--- Plan ---", "# My Plan"],
        &[]
    },
    plan_without_context = {
        EscalationTrigger::Plan { last_message: None },
        &["requesting plan approval"],
        &["--- Plan ---"]
    },
)]
fn trigger_context_fragments(
    trigger: EscalationTrigger,
    must_contain: &[&str],
    must_not_contain: &[&str],
) {
    let d = build_job_decision(trigger);
    for s in must_contain {
        assert!(d.context.contains(s), "context should contain '{}': {}", s, d.context);
    }
    for s in must_not_contain {
        assert!(!d.context.contains(s), "context should not contain '{}': {}", s, d.context);
    }
}

#[test]
fn test_builder_with_agent_id_and_project() {
    let d = unwrap_decision(
        EscalationDecisionBuilder::new(
            JobId::new("job-1").into(),
            "test-job".to_string(),
            "agent-123".to_string(),
            EscalationTrigger::Idle { last_message: None },
        )
        .project("my-project")
        .build(),
    );
    assert_eq!(d.agent_id, AgentId::new("agent-123"));
    assert_eq!(d.project, "my-project");
}

#[test]
fn test_builder_with_agent_log_tail() {
    let d = unwrap_decision(
        EscalationDecisionBuilder::new(
            JobId::new("job-1").into(),
            "test-job".to_string(),
            "agent-1".to_string(),
            EscalationTrigger::Idle { last_message: None },
        )
        .agent_log_tail("last few lines of output")
        .build(),
    );
    assert!(d.context.contains("Recent agent output:"));
    assert!(d.context.contains("last few lines of output"));
}

#[yare::parameterized(
    with_data = {
        Some(QuestionData {
            questions: vec![QuestionEntry {
                question: "Which library should we use?".to_string(),
                header: Some("Library".to_string()),
                options: vec![
                    QuestionOption { label: "React".to_string(), description: Some("Popular UI library".to_string()) },
                    QuestionOption { label: "Vue".to_string(), description: Some("Progressive framework".to_string()) },
                ],
                multi_select: false,
            }],
        }),
        &["React", "Vue", "Other", "Cancel", "Dismiss"],
        &["Which library should we use?", "[Library]"],
        Some(1)
    },
    without_data = {
        None,
        &["Other", "Cancel", "Dismiss"],
        &["no details available"],
        None
    },
    multi_question_context = {
        Some(QuestionData {
            questions: vec![
                QuestionEntry {
                    question: "First question?".to_string(),
                    header: Some("Q1".to_string()),
                    options: vec![QuestionOption { label: "Yes".to_string(), description: None }],
                    multi_select: false,
                },
                QuestionEntry {
                    question: "Second question?".to_string(),
                    header: Some("Q2".to_string()),
                    options: vec![],
                    multi_select: false,
                },
            ],
        }),
        &["Yes", "Other", "Cancel", "Dismiss"],
        &["[Q1] First question?", "[Q2] Second question?"],
        Some(2)
    },
    multi_question_all_options = {
        Some(QuestionData {
            questions: vec![
                QuestionEntry {
                    question: "Which framework?".to_string(),
                    header: Some("Framework".to_string()),
                    options: vec![
                        QuestionOption { label: "React".to_string(), description: Some("Component-based".to_string()) },
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
        }),
        &["React", "Vue", "PostgreSQL", "MySQL", "Other", "Cancel", "Dismiss"],
        &["Which framework?", "Which database?"],
        Some(2)
    },
)]
fn question_trigger_builds_options_and_context(
    questions: Option<QuestionData>,
    expected_labels: &[&str],
    expected_context: &[&str],
    expected_question_count: Option<usize>,
) {
    let d = build_job_decision(EscalationTrigger::Question { questions, last_message: None });
    assert_eq!(d.source, DecisionSource::Question);
    assert_eq!(d.options.len(), expected_labels.len());
    for (opt, label) in d.options.iter().zip(expected_labels) {
        assert_eq!(opt.label, *label);
    }
    for frag in expected_context {
        assert!(d.context.contains(frag), "context should contain '{}'", frag);
    }
    match expected_question_count {
        Some(n) => {
            let qd = d.questions.expect("questions should be present");
            assert_eq!(qd.questions.len(), n);
        }
        None => assert!(d.questions.is_none()),
    }
}

#[yare::parameterized(
    idle = {
        "run-123", "my-command",
        EscalationTrigger::Idle { last_message: None },
        DecisionSource::Idle, 4, "Nudge",
        None, None, &[]
    },
    error = {
        "run-456", "build-project",
        EscalationTrigger::Error { error_type: "OutOfCredits".into(), message: "API quota exceeded".into(), last_message: None },
        DecisionSource::Error, 4, "Retry",
        Some("test-ns"), None, &["OutOfCredits"]
    },
    dead_with_agent_id = {
        "run-001", "deploy",
        EscalationTrigger::Dead { exit_code: Some(1), last_message: None },
        DecisionSource::Dead, 4, "Retry",
        None, Some("agent-uuid-123"), &[]
    },
    plan = {
        "run-plan", "my-planner",
        EscalationTrigger::Plan { last_message: Some("Plan content here".into()) },
        DecisionSource::Plan, 5, "Accept (clear context)",
        Some("test-ns"), None, &["Plan content here"]
    },
)]
fn for_crew_properties(
    run_id_str: &str,
    agent_name: &str,
    trigger: EscalationTrigger,
    expected_source: DecisionSource,
    expected_option_count: usize,
    expected_first_label: &str,
    project: Option<&str>,
    agent_id: Option<&str>,
    context_fragments: &[&str],
) {
    let aid = agent_id.unwrap_or("agent-1");
    let mut builder = EscalationDecisionBuilder::new(
        CrewId::new(run_id_str).into(),
        agent_name.to_string(),
        aid.to_string(),
        trigger,
    );
    if let Some(ns) = project {
        builder = builder.project(ns);
    }
    let d = unwrap_decision(builder.build());
    assert_eq!(d.owner, OwnerId::Crew(CrewId::new(run_id_str)));
    assert_eq!(d.source, expected_source);
    assert_eq!(d.options.len(), expected_option_count);
    assert_eq!(d.options[0].label, expected_first_label);
    if let Some(ns) = project {
        assert_eq!(d.project, ns);
    }
    assert_eq!(d.agent_id, AgentId::new(aid));
    assert!(d.context.contains(agent_name));
    for frag in context_fragments {
        assert!(d.context.contains(frag), "context should contain '{}': {}", frag, d.context);
    }
}

#[test]
fn test_for_job_creates_job_owner() {
    let d = unwrap_decision(
        EscalationDecisionBuilder::new(
            JobId::new("job-789").into(),
            "test-job".to_string(),
            "agent-1".to_string(),
            EscalationTrigger::Idle { last_message: None },
        )
        .build(),
    );
    assert_eq!(d.owner, OwnerId::Job(JobId::new("job-789")));
}

fn arb_escalation_trigger() -> impl Strategy<Value = EscalationTrigger> {
    prop_oneof![
        any::<Option<String>>().prop_map(|ac| EscalationTrigger::Idle { last_message: ac }),
        (any::<Option<i32>>(), any::<Option<String>>())
            .prop_map(|(ec, ac)| { EscalationTrigger::Dead { exit_code: ec, last_message: ac } }),
        ("[a-zA-Z]{1,10}", "[a-zA-Z ]{0,20}", any::<Option<String>>()).prop_map(|(et, msg, ac)| {
            EscalationTrigger::Error { error_type: et, message: msg, last_message: ac }
        }),
        ("[a-zA-Z./]{1,20}", 1..255i32, "[a-zA-Z ]{0,30}").prop_map(|(cmd, ec, stderr)| {
            EscalationTrigger::GateFailed { command: cmd, exit_code: ec, stderr }
        }),
        ("[a-zA-Z]{1,10}", any::<Option<String>>()).prop_map(|(pt, ac)| {
            EscalationTrigger::Prompt { prompt_type: pt, last_message: ac }
        }),
        any::<Option<String>>()
            .prop_map(|ac| EscalationTrigger::Question { questions: None, last_message: ac }),
        any::<Option<String>>().prop_map(|ac| EscalationTrigger::Plan { last_message: ac }),
    ]
}

proptest! {
    /// Every trigger type maps to its expected DecisionSource variant.
    #[test]
    fn trigger_source_is_consistent(trigger in arb_escalation_trigger()) {
        let source = trigger.to_source();
        match &trigger {
            EscalationTrigger::Idle { .. } => prop_assert_eq!(source, DecisionSource::Idle),
            EscalationTrigger::Dead { .. } => prop_assert_eq!(source, DecisionSource::Dead),
            EscalationTrigger::Error { .. } => prop_assert_eq!(source, DecisionSource::Error),
            EscalationTrigger::GateFailed { .. } => prop_assert_eq!(source, DecisionSource::Gate),
            EscalationTrigger::Prompt { .. } => prop_assert_eq!(source, DecisionSource::Approval),
            EscalationTrigger::Question { .. } => prop_assert_eq!(source, DecisionSource::Question),
            EscalationTrigger::Plan { .. } => prop_assert_eq!(source, DecisionSource::Plan),
        }
    }

    /// Every trigger produces at least 3 options (all have Cancel/Dismiss or equivalent).
    #[test]
    fn trigger_always_produces_options(trigger in arb_escalation_trigger()) {
        let d = build_job_decision(trigger);
        prop_assert!(d.options.len() >= 3, "expected >=3 options, got {}", d.options.len());
    }

    /// Every trigger produces non-empty context.
    #[test]
    fn trigger_always_produces_context(trigger in arb_escalation_trigger()) {
        let d = build_job_decision(trigger);
        prop_assert!(!d.context.is_empty(), "context should not be empty");
    }

    /// Decision ID is always non-empty (UUID).
    #[test]
    fn trigger_always_produces_id(trigger in arb_escalation_trigger()) {
        let d = build_job_decision(trigger);
        prop_assert!(!d.id.as_str().is_empty());
    }

    /// for_job always sets owner to Job variant.
    #[test]
    fn for_job_owner_is_job(trigger in arb_escalation_trigger()) {
        let d = build_job_decision(trigger);
        prop_assert!(matches!(d.owner, OwnerId::Job(_)));
    }

    /// for_crew always sets owner to Crew variant with correct id.
    #[test]
    fn for_crew_owner_is_crew(trigger in arb_escalation_trigger()) {
        let d = unwrap_decision(
            EscalationDecisionBuilder::new(
                CrewId::new("run-prop").into(),
                "test-cmd".to_string(),
                "agent-1".to_string(),
                trigger,
            )
            .build(),
        );
        prop_assert_eq!(d.owner, OwnerId::Crew(CrewId::new("run-prop")));
    }
}
