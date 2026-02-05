// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::{
    AgentRunId, DecisionSource, Event, JobId, OwnerId, QuestionData, QuestionEntry, QuestionOption,
};

#[test]
fn test_idle_trigger_builds_correct_options() {
    let (id, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Idle,
    )
    .build();

    assert!(!id.is_empty());

    match event {
        Event::DecisionCreated {
            options, source, ..
        } => {
            assert_eq!(source, DecisionSource::Idle);
            assert_eq!(options.len(), 4);
            assert_eq!(options[0].label, "Nudge");
            assert!(options[0].recommended);
            assert_eq!(options[1].label, "Done");
            assert!(!options[1].recommended);
            assert_eq!(options[2].label, "Cancel");
            assert!(!options[2].recommended);
            assert_eq!(options[3].label, "Dismiss");
            assert!(!options[3].recommended);
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_dead_trigger_builds_correct_options() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Dead {
            exit_code: Some(137),
        },
    )
    .build();

    match event {
        Event::DecisionCreated {
            options,
            source,
            context,
            ..
        } => {
            assert_eq!(source, DecisionSource::Error);
            assert_eq!(options.len(), 3);
            assert_eq!(options[0].label, "Retry");
            assert!(options[0].recommended);
            assert_eq!(options[1].label, "Skip");
            assert_eq!(options[2].label, "Cancel");
            assert!(context.contains("exit code 137"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_error_trigger_builds_correct_options() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Error {
            error_type: "OutOfCredits".to_string(),
            message: "API quota exceeded".to_string(),
        },
    )
    .build();

    match event {
        Event::DecisionCreated {
            options,
            source,
            context,
            ..
        } => {
            assert_eq!(source, DecisionSource::Error);
            assert_eq!(options.len(), 3);
            assert_eq!(options[0].label, "Retry");
            assert!(context.contains("OutOfCredits"));
            assert!(context.contains("API quota exceeded"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_gate_failure_includes_command_and_stderr() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::GateFailed {
            command: "./check.sh".to_string(),
            exit_code: 1,
            stderr: "validation failed".to_string(),
        },
    )
    .build();

    match event {
        Event::DecisionCreated {
            context, source, ..
        } => {
            assert_eq!(source, DecisionSource::Gate);
            assert!(context.contains("./check.sh"));
            assert!(context.contains("validation failed"));
            assert!(context.contains("Exit code: 1"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_prompt_trigger_builds_correct_options() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Prompt {
            prompt_type: "permission".to_string(),
        },
    )
    .build();

    match event {
        Event::DecisionCreated {
            options,
            source,
            context,
            ..
        } => {
            assert_eq!(source, DecisionSource::Approval);
            assert_eq!(options.len(), 3);
            assert_eq!(options[0].label, "Approve");
            assert_eq!(options[1].label, "Deny");
            assert_eq!(options[2].label, "Cancel");
            assert!(context.contains("permission prompt"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_builder_with_agent_id_and_namespace() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Idle,
    )
    .agent_id("agent-123")
    .namespace("my-project")
    .build();

    match event {
        Event::DecisionCreated {
            agent_id,
            namespace,
            ..
        } => {
            assert_eq!(agent_id, Some("agent-123".to_string()));
            assert_eq!(namespace, "my-project");
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_builder_with_agent_log_tail() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Idle,
    )
    .agent_log_tail("last few lines of output")
    .build();

    match event {
        Event::DecisionCreated { context, .. } => {
            assert!(context.contains("Recent agent output:"));
            assert!(context.contains("last few lines of output"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_dead_trigger_without_exit_code() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Dead { exit_code: None },
    )
    .build();

    match event {
        Event::DecisionCreated { context, .. } => {
            assert!(context.contains("exited unexpectedly"));
            assert!(!context.contains("exit code"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_gate_failure_empty_stderr() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::GateFailed {
            command: "./check.sh".to_string(),
            exit_code: 1,
            stderr: String::new(),
        },
    )
    .build();

    match event {
        Event::DecisionCreated { context, .. } => {
            assert!(context.contains("./check.sh"));
            assert!(context.contains("Exit code: 1"));
            assert!(!context.contains("stderr:"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_question_trigger_with_data() {
    let question_data = QuestionData {
        questions: vec![QuestionEntry {
            question: "Which library should we use?".to_string(),
            header: Some("Library".to_string()),
            options: vec![
                QuestionOption {
                    label: "React".to_string(),
                    description: Some("Popular UI library".to_string()),
                },
                QuestionOption {
                    label: "Vue".to_string(),
                    description: Some("Progressive framework".to_string()),
                },
            ],
            multi_select: false,
        }],
    };

    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Question {
            question_data: Some(question_data),
        },
    )
    .build();

    match event {
        Event::DecisionCreated {
            options,
            source,
            context,
            ..
        } => {
            assert_eq!(source, DecisionSource::Question);
            // 2 user options + Cancel
            assert_eq!(options.len(), 3);
            assert_eq!(options[0].label, "React");
            assert_eq!(
                options[0].description,
                Some("Popular UI library".to_string())
            );
            assert_eq!(options[1].label, "Vue");
            assert_eq!(options[2].label, "Cancel");
            // Context includes question text
            assert!(context.contains("Which library should we use?"));
            assert!(context.contains("[Library]"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_question_trigger_without_data() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Question {
            question_data: None,
        },
    )
    .build();

    match event {
        Event::DecisionCreated {
            options,
            source,
            context,
            ..
        } => {
            assert_eq!(source, DecisionSource::Question);
            // Only Cancel when no question data
            assert_eq!(options.len(), 1);
            assert_eq!(options[0].label, "Cancel");
            assert!(context.contains("no details available"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_question_trigger_maps_to_question_source() {
    let trigger = EscalationTrigger::Question {
        question_data: None,
    };
    assert_eq!(trigger.to_source(), DecisionSource::Question);
}

#[test]
fn test_question_trigger_multi_question_context() {
    let question_data = QuestionData {
        questions: vec![
            QuestionEntry {
                question: "First question?".to_string(),
                header: Some("Q1".to_string()),
                options: vec![QuestionOption {
                    label: "Yes".to_string(),
                    description: None,
                }],
                multi_select: false,
            },
            QuestionEntry {
                question: "Second question?".to_string(),
                header: Some("Q2".to_string()),
                options: vec![],
                multi_select: false,
            },
        ],
    };

    #[allow(deprecated)]
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Question {
            question_data: Some(question_data),
        },
    )
    .build();

    match event {
        Event::DecisionCreated {
            context, options, ..
        } => {
            assert!(context.contains("[Q1] First question?"));
            assert!(context.contains("[Q2] Second question?"));
            // Options come from first question only
            assert_eq!(options.len(), 2); // "Yes" + "Cancel"
            assert_eq!(options[0].label, "Yes");
            assert_eq!(options[1].label, "Cancel");
        }
        _ => panic!("expected DecisionCreated"),
    }
}

// ===================== Tests for for_agent_run() =====================

#[test]
fn test_for_agent_run_idle_trigger() {
    let (id, event) = EscalationDecisionBuilder::for_agent_run(
        AgentRunId::new("ar-123"),
        "my-command".to_string(),
        EscalationTrigger::Idle,
    )
    .build();

    assert!(!id.is_empty());

    match event {
        Event::DecisionCreated {
            job_id,
            owner,
            source,
            options,
            context,
            ..
        } => {
            // job_id should be empty for agent runs
            assert!(job_id.as_str().is_empty());
            // owner should be AgentRun
            assert_eq!(owner, Some(OwnerId::AgentRun(AgentRunId::new("ar-123"))));
            assert_eq!(source, DecisionSource::Idle);
            assert_eq!(options.len(), 4);
            assert_eq!(options[0].label, "Nudge");
            // Context should use the command name
            assert!(context.contains("my-command"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_for_agent_run_error_trigger() {
    let (_, event) = EscalationDecisionBuilder::for_agent_run(
        AgentRunId::new("ar-456"),
        "build-project".to_string(),
        EscalationTrigger::Error {
            error_type: "OutOfCredits".to_string(),
            message: "API quota exceeded".to_string(),
        },
    )
    .namespace("test-ns")
    .build();

    match event {
        Event::DecisionCreated {
            owner,
            source,
            options,
            namespace,
            context,
            ..
        } => {
            assert_eq!(owner, Some(OwnerId::AgentRun(AgentRunId::new("ar-456"))));
            assert_eq!(source, DecisionSource::Error);
            assert_eq!(options.len(), 3);
            assert_eq!(options[0].label, "Retry");
            assert_eq!(namespace, "test-ns");
            assert!(context.contains("OutOfCredits"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_for_job_creates_job_owner() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("job-789"),
        "test-job".to_string(),
        EscalationTrigger::Idle,
    )
    .build();

    match event {
        Event::DecisionCreated { job_id, owner, .. } => {
            assert_eq!(job_id.as_str(), "job-789");
            assert_eq!(owner, Some(OwnerId::Job(JobId::new("job-789"))));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_for_agent_run_with_agent_id() {
    let (_, event) = EscalationDecisionBuilder::for_agent_run(
        AgentRunId::new("ar-001"),
        "deploy".to_string(),
        EscalationTrigger::Dead { exit_code: Some(1) },
    )
    .agent_id("agent-uuid-123")
    .build();

    match event {
        Event::DecisionCreated {
            agent_id, owner, ..
        } => {
            assert_eq!(agent_id, Some("agent-uuid-123".to_string()));
            assert_eq!(owner, Some(OwnerId::AgentRun(AgentRunId::new("ar-001"))));
        }
        _ => panic!("expected DecisionCreated"),
    }
}
