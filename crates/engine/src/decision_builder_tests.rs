// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::{DecisionSource, Event, PipelineId};

#[test]
fn test_idle_trigger_builds_correct_options() {
    let (id, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
        EscalationTrigger::Idle,
    )
    .build();

    assert!(!id.is_empty());

    match event {
        Event::DecisionCreated {
            options, source, ..
        } => {
            assert_eq!(source, DecisionSource::Idle);
            assert_eq!(options.len(), 3);
            assert_eq!(options[0].label, "Nudge");
            assert!(options[0].recommended);
            assert_eq!(options[1].label, "Done");
            assert!(!options[1].recommended);
            assert_eq!(options[2].label, "Cancel");
            assert!(!options[2].recommended);
        }
        _ => panic!("expected DecisionCreated"),
    }
}

#[test]
fn test_dead_trigger_builds_correct_options() {
    let (_, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
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
    let (_, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
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
    let (_, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
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
    let (_, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
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
    let (_, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
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
    let (_, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
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
    let (_, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
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
    let (_, event) = EscalationDecisionBuilder::new(
        PipelineId::new("pipe-1"),
        "test-pipeline".to_string(),
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
