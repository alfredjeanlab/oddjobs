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
        EscalationTrigger::Idle {
            assistant_context: None,
        },
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
            assistant_context: None,
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
            assert_eq!(source, DecisionSource::Dead);
            assert_eq!(options.len(), 4);
            assert_eq!(options[0].label, "Retry");
            assert!(options[0].recommended);
            assert_eq!(options[1].label, "Skip");
            assert_eq!(options[2].label, "Cancel");
            assert_eq!(options[3].label, "Dismiss");
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
            assistant_context: None,
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
            assert_eq!(options.len(), 4);
            assert_eq!(options[0].label, "Retry");
            assert_eq!(options[3].label, "Dismiss");
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
            assistant_context: None,
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
            assert_eq!(options.len(), 4);
            assert_eq!(options[0].label, "Approve");
            assert_eq!(options[1].label, "Deny");
            assert_eq!(options[2].label, "Cancel");
            assert_eq!(options[3].label, "Dismiss");
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
        EscalationTrigger::Idle {
            assistant_context: None,
        },
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
        EscalationTrigger::Idle {
            assistant_context: None,
        },
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
        EscalationTrigger::Dead {
            exit_code: None,
            assistant_context: None,
        },
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
            assistant_context: None,
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
            // 2 user options + Other + Cancel + Dismiss
            assert_eq!(options.len(), 5);
            assert_eq!(options[0].label, "React");
            assert_eq!(
                options[0].description,
                Some("Popular UI library".to_string())
            );
            assert_eq!(options[1].label, "Vue");
            assert_eq!(options[2].label, "Other");
            assert_eq!(options[3].label, "Cancel");
            assert_eq!(options[4].label, "Dismiss");
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
            assistant_context: None,
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
            // Other + Cancel + Dismiss when no question data
            assert_eq!(options.len(), 3);
            assert_eq!(options[0].label, "Other");
            assert_eq!(options[1].label, "Cancel");
            assert_eq!(options[2].label, "Dismiss");
            assert!(context.contains("no details available"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_question_trigger_maps_to_question_source() {
    let trigger = EscalationTrigger::Question {
        question_data: None,
        assistant_context: None,
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
            assistant_context: None,
        },
    )
    .build();

    match event {
        Event::DecisionCreated {
            context,
            options,
            question_data,
            ..
        } => {
            assert!(context.contains("[Q1] First question?"));
            assert!(context.contains("[Q2] Second question?"));
            // Options come from ALL questions
            assert_eq!(options.len(), 4); // "Yes" (from Q1) + "Other" + "Cancel" + "Dismiss"
            assert_eq!(options[0].label, "Yes");
            assert_eq!(options[1].label, "Other");
            assert_eq!(options[2].label, "Cancel");
            assert_eq!(options[3].label, "Dismiss");
            // question_data is passed through
            assert!(question_data.is_some());
            assert_eq!(question_data.unwrap().questions.len(), 2);
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_multi_question_options_from_all_questions() {
    let question_data = QuestionData {
        questions: vec![
            QuestionEntry {
                question: "Which framework?".to_string(),
                header: Some("Framework".to_string()),
                options: vec![
                    QuestionOption {
                        label: "React".to_string(),
                        description: Some("Component-based".to_string()),
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
    };

    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Question {
            question_data: Some(question_data),
            assistant_context: None,
        },
    )
    .build();

    match event {
        Event::DecisionCreated {
            options,
            question_data,
            ..
        } => {
            // Options from ALL questions: React, Vue, PostgreSQL, MySQL + Other + Cancel + Dismiss
            assert_eq!(options.len(), 7);
            assert_eq!(options[0].label, "React");
            assert_eq!(options[0].description, Some("Component-based".to_string()));
            assert_eq!(options[1].label, "Vue");
            assert_eq!(options[2].label, "PostgreSQL");
            assert_eq!(options[3].label, "MySQL");
            assert_eq!(options[4].label, "Other");
            assert_eq!(options[5].label, "Cancel");
            assert_eq!(options[6].label, "Dismiss");

            // question_data preserved
            let qd = question_data.unwrap();
            assert_eq!(qd.questions.len(), 2);
            assert_eq!(qd.questions[0].question, "Which framework?");
            assert_eq!(qd.questions[1].question, "Which database?");
        }
        _ => panic!("expected DecisionCreated"),
    }
}

// ===================== Tests for Signal trigger =====================

#[test]
fn test_signal_trigger_builds_correct_options() {
    let (id, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Signal {
            message: "Need human help with merge conflicts".to_string(),
        },
    )
    .build();

    assert!(!id.is_empty());

    match event {
        Event::DecisionCreated {
            options,
            source,
            context,
            ..
        } => {
            assert_eq!(source, DecisionSource::Signal);
            assert_eq!(options.len(), 4);
            assert_eq!(options[0].label, "Nudge");
            assert!(options[0].recommended);
            assert_eq!(options[1].label, "Done");
            assert_eq!(options[2].label, "Cancel");
            assert_eq!(options[3].label, "Dismiss");
            assert!(context.contains("requested escalation"));
            assert!(context.contains("Need human help with merge conflicts"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_signal_trigger_maps_to_signal_source() {
    let trigger = EscalationTrigger::Signal {
        message: "help".to_string(),
    };
    assert_eq!(trigger.to_source(), DecisionSource::Signal);
}

#[test]
fn test_signal_trigger_for_agent_run() {
    let (_, event) = EscalationDecisionBuilder::for_agent_run(
        AgentRunId::new("ar-sig"),
        "my-worker".to_string(),
        EscalationTrigger::Signal {
            message: "Stuck on tests".to_string(),
        },
    )
    .namespace("test-ns")
    .build();

    match event {
        Event::DecisionCreated {
            owner,
            source,
            context,
            namespace,
            ..
        } => {
            assert_eq!(owner, OwnerId::AgentRun(AgentRunId::new("ar-sig")));
            assert_eq!(source, DecisionSource::Signal);
            assert_eq!(namespace, "test-ns");
            assert!(context.contains("my-worker"));
            assert!(context.contains("Stuck on tests"));
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
        EscalationTrigger::Idle {
            assistant_context: None,
        },
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
            assert_eq!(owner, OwnerId::AgentRun(AgentRunId::new("ar-123")));
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
            assistant_context: None,
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
            assert_eq!(owner, OwnerId::AgentRun(AgentRunId::new("ar-456")));
            assert_eq!(source, DecisionSource::Error);
            assert_eq!(options.len(), 4);
            assert_eq!(options[0].label, "Retry");
            assert_eq!(options[3].label, "Dismiss");
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
        EscalationTrigger::Idle {
            assistant_context: None,
        },
    )
    .build();

    match event {
        Event::DecisionCreated { job_id, owner, .. } => {
            assert_eq!(job_id.as_str(), "job-789");
            assert_eq!(owner, OwnerId::Job(JobId::new("job-789")));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_for_agent_run_with_agent_id() {
    let (_, event) = EscalationDecisionBuilder::for_agent_run(
        AgentRunId::new("ar-001"),
        "deploy".to_string(),
        EscalationTrigger::Dead {
            exit_code: Some(1),
            assistant_context: None,
        },
    )
    .agent_id("agent-uuid-123")
    .build();

    match event {
        Event::DecisionCreated {
            agent_id, owner, ..
        } => {
            assert_eq!(agent_id, Some("agent-uuid-123".to_string()));
            assert_eq!(owner, OwnerId::AgentRun(AgentRunId::new("ar-001")));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

// ===================== Tests for Plan trigger =====================

#[test]
fn test_plan_trigger_builds_correct_options() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Plan {
            assistant_context: Some("# My Plan\n\n## Steps\n1. Do thing".to_string()),
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
            assert_eq!(source, DecisionSource::Plan);
            assert_eq!(options.len(), 5);
            assert_eq!(options[0].label, "Accept (clear context)");
            assert!(options[0].recommended);
            assert_eq!(options[1].label, "Accept (auto edits)");
            assert!(!options[1].recommended);
            assert_eq!(options[2].label, "Accept (manual edits)");
            assert_eq!(options[3].label, "Revise");
            assert_eq!(options[4].label, "Cancel");
            // Context should include plan content under "Plan" label
            assert!(context.contains("requesting plan approval"));
            assert!(context.contains("--- Plan ---"));
            assert!(context.contains("# My Plan"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_plan_trigger_maps_to_plan_source() {
    let trigger = EscalationTrigger::Plan {
        assistant_context: None,
    };
    assert_eq!(trigger.to_source(), DecisionSource::Plan);
}

#[test]
fn test_plan_trigger_without_context() {
    let (_, event) = EscalationDecisionBuilder::for_job(
        JobId::new("pipe-1"),
        "test-job".to_string(),
        EscalationTrigger::Plan {
            assistant_context: None,
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
            assert_eq!(source, DecisionSource::Plan);
            assert_eq!(options.len(), 5);
            assert!(context.contains("requesting plan approval"));
            // No plan content section when assistant_context is None
            assert!(!context.contains("--- Plan ---"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_plan_trigger_for_agent_run() {
    let (_, event) = EscalationDecisionBuilder::for_agent_run(
        AgentRunId::new("ar-plan"),
        "my-planner".to_string(),
        EscalationTrigger::Plan {
            assistant_context: Some("Plan content here".to_string()),
        },
    )
    .namespace("test-ns")
    .build();

    match event {
        Event::DecisionCreated {
            owner,
            source,
            options,
            context,
            namespace,
            ..
        } => {
            assert_eq!(owner, OwnerId::AgentRun(AgentRunId::new("ar-plan")));
            assert_eq!(source, DecisionSource::Plan);
            assert_eq!(options.len(), 5);
            assert_eq!(namespace, "test-ns");
            assert!(context.contains("my-planner"));
            assert!(context.contains("Plan content here"));
        }
        _ => panic!("expected DecisionCreated"),
    }
}
