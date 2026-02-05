// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::agent::AgentError;

#[test]
fn event_serialization_roundtrip() {
    let events = vec![
        Event::CommandRun {
            job_id: JobId::new("pipe-1"),
            job_name: "build".to_string(),
            project_root: std::path::PathBuf::from("/test/project"),
            invoke_dir: std::path::PathBuf::from("/test/project"),
            command: "build".to_string(),
            args: [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            namespace: String::new(),
        },
        Event::AgentWaiting {
            agent_id: AgentId::new("agent-1"),
            owner: None,
        },
        Event::AgentFailed {
            agent_id: AgentId::new("agent-2"),
            error: AgentError::RateLimited,
            owner: None,
        },
        Event::ShellExited {
            job_id: JobId::new("pipe-1"),
            step: "init".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        },
    ];

    for event in events {
        let json = serde_json::to_string(&event).unwrap();
        let parsed: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(event, parsed);
    }
}

#[test]
fn event_json_format_shutdown() {
    let event = Event::Shutdown;
    let json = serde_json::to_string(&event).unwrap();
    assert_eq!(json, r#"{"type":"system:shutdown"}"#);
}

#[test]
fn event_unknown_type_becomes_custom() {
    let json = r#"{"type":"unknown:event","foo":"bar"}"#;
    let parsed: Event = serde_json::from_str(json).unwrap();
    assert_eq!(parsed, Event::Custom);
}

#[test]
fn event_agent_working_roundtrip() {
    let event = Event::AgentWorking {
        agent_id: AgentId::new("a1"),
        owner: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:working");
    assert_eq!(json["agent_id"], "a1");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_agent_failed_roundtrip() {
    let event = Event::AgentFailed {
        agent_id: AgentId::new("a2"),
        error: AgentError::RateLimited,
        owner: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:failed");
    assert_eq!(json["agent_id"], "a2");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_agent_exited_roundtrip() {
    let event = Event::AgentExited {
        agent_id: AgentId::new("a3"),
        exit_code: Some(42),
        owner: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:exited");
    assert_eq!(json["exit_code"], 42);

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_agent_exited_no_code_roundtrip() {
    let event = Event::AgentExited {
        agent_id: AgentId::new("a4"),
        exit_code: None,
        owner: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:exited");
    assert!(json["exit_code"].is_null());

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_agent_gone_roundtrip() {
    let event = Event::AgentGone {
        agent_id: AgentId::new("a5"),
        owner: None,
    };
    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
    let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(json["type"], "agent:gone");
}

#[test]
fn event_job_created_roundtrip() {
    let event = Event::JobCreated {
        id: JobId::new("pipe-1"),
        kind: "build".to_string(),
        name: "test".to_string(),
        runbook_hash: "abc123".to_string(),
        cwd: PathBuf::from("/test/project"),
        vars: [("name".to_string(), "test".to_string())]
            .into_iter()
            .collect(),
        initial_step: "init".to_string(),
        created_at_epoch_ms: 1_000_000,
        namespace: String::new(),
        cron_name: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "job:created");
    assert_eq!(json["id"], "pipe-1");
    assert_eq!(json["kind"], "build");
    assert_eq!(json["initial_step"], "init");
    assert_eq!(json["created_at_epoch_ms"], 1_000_000);

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_step_started_roundtrip() {
    let event = Event::StepStarted {
        job_id: JobId::new("pipe-1"),
        step: "build".to_string(),
        agent_id: None,
        agent_name: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "step:started");
    assert_eq!(json["job_id"], "pipe-1");
    assert_eq!(json["step"], "build");
    assert!(json.get("agent_id").is_none());

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_step_waiting_roundtrip() {
    let event = Event::StepWaiting {
        job_id: JobId::new("pipe-1"),
        step: "review".to_string(),
        reason: Some("gate failed".to_string()),
        decision_id: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "step:waiting");
    assert_eq!(json["job_id"], "pipe-1");
    assert_eq!(json["step"], "review");
    assert_eq!(json["reason"], "gate failed");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_step_completed_roundtrip() {
    let event = Event::StepCompleted {
        job_id: JobId::new("pipe-1"),
        step: "deploy".to_string(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "step:completed");
    assert_eq!(json["job_id"], "pipe-1");
    assert_eq!(json["step"], "deploy");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_step_failed_roundtrip() {
    let event = Event::StepFailed {
        job_id: JobId::new("pipe-1"),
        step: "test".to_string(),
        error: "something went wrong".to_string(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "step:failed");
    assert_eq!(json["job_id"], "pipe-1");
    assert_eq!(json["step"], "test");
    assert_eq!(json["error"], "something went wrong");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_job_resume_roundtrip() {
    let event = Event::JobResume {
        id: JobId::new("pipe-1"),
        message: Some("try again".to_string()),
        vars: [("key".to_string(), "value".to_string())]
            .into_iter()
            .collect(),
        kill: false,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "job:resume");
    assert_eq!(json["id"], "pipe-1");
    assert_eq!(json["message"], "try again");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_job_resume_no_message_roundtrip() {
    let event = Event::JobResume {
        id: JobId::new("pipe-1"),
        message: None,
        vars: HashMap::new(),
        kill: false,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "job:resume");
    assert!(json.get("message").is_none());

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_workspace_drop_roundtrip() {
    let event = Event::WorkspaceDrop {
        id: WorkspaceId::new("ws-1"),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "workspace:drop");
    assert_eq!(json["id"], "ws-1");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_workspace_ready_roundtrip() {
    let event = Event::WorkspaceReady {
        id: WorkspaceId::new("ws-1"),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "workspace:ready");
    assert_eq!(json["id"], "ws-1");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_workspace_failed_roundtrip() {
    let event = Event::WorkspaceFailed {
        id: WorkspaceId::new("ws-1"),
        reason: "disk full".to_string(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "workspace:failed");
    assert_eq!(json["id"], "ws-1");
    assert_eq!(json["reason"], "disk full");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_job_id_returns_id_for_job_events() {
    let cases: Vec<(Event, JobId)> = vec![
        (
            Event::CommandRun {
                job_id: JobId::new("p1"),
                job_name: "b".to_string(),
                project_root: PathBuf::from("/"),
                invoke_dir: PathBuf::from("/"),
                command: "build".to_string(),
                args: HashMap::new(),
                namespace: String::new(),
            },
            JobId::new("p1"),
        ),
        (
            Event::JobCreated {
                id: JobId::new("p6"),
                kind: "build".to_string(),
                name: "test".to_string(),
                runbook_hash: "abc".to_string(),
                cwd: PathBuf::from("/"),
                vars: HashMap::new(),
                initial_step: "init".to_string(),
                created_at_epoch_ms: 1_000_000,
                namespace: String::new(),
                cron_name: None,
            },
            JobId::new("p6"),
        ),
    ];

    for (event, expected_id) in cases {
        assert_eq!(
            event.job_id(),
            Some(&expected_id),
            "wrong job_id for {:?}",
            event
        );
    }
}

#[test]
fn event_job_id_returns_none_for_non_job_events() {
    let events = vec![
        Event::TimerStart {
            id: TimerId::new("t"),
        },
        Event::SessionDeleted {
            id: SessionId::new("s"),
        },
        Event::Custom,
        Event::Shutdown,
    ];

    for event in events {
        assert_eq!(event.job_id(), None, "expected None for {:?}", event);
    }
}

#[test]
fn event_from_agent_state() {
    let agent_id = AgentId::new("test");

    assert!(matches!(
        Event::from_agent_state(agent_id.clone(), AgentState::Working, None),
        Event::AgentWorking { .. }
    ));
    assert!(matches!(
        Event::from_agent_state(agent_id.clone(), AgentState::WaitingForInput, None),
        Event::AgentWaiting { .. }
    ));
    assert!(matches!(
        Event::from_agent_state(agent_id.clone(), AgentState::SessionGone, None),
        Event::AgentGone { .. }
    ));
}

#[test]
fn event_as_agent_state() {
    let agent_id = AgentId::new("test");

    let event = Event::AgentWorking {
        agent_id: agent_id.clone(),
        owner: None,
    };
    let (id, state, _owner) = event.as_agent_state().unwrap();
    assert_eq!(id, &agent_id);
    assert!(matches!(state, AgentState::Working));

    let event = Event::Shutdown;
    assert!(event.as_agent_state().is_none());
}

#[test]
fn event_queue_pushed_roundtrip() {
    let event = Event::QueuePushed {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        data: [("title".to_string(), "Fix bug".to_string())]
            .into_iter()
            .collect(),
        pushed_at_epoch_ms: 1_000_000,
        namespace: String::new(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "queue:pushed");
    assert_eq!(json["queue_name"], "bugs");
    assert_eq!(json["item_id"], "item-1");
    assert_eq!(json["pushed_at_epoch_ms"], 1_000_000);

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_queue_taken_roundtrip() {
    let event = Event::QueueTaken {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        worker_name: "fixer".to_string(),
        namespace: String::new(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "queue:taken");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_queue_completed_roundtrip() {
    let event = Event::QueueCompleted {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        namespace: String::new(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "queue:completed");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_queue_failed_roundtrip() {
    let event = Event::QueueFailed {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        error: "job failed".to_string(),
        namespace: String::new(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "queue:failed");
    assert_eq!(json["error"], "job failed");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_queue_name_returns_correct_strings() {
    assert_eq!(
        Event::QueuePushed {
            queue_name: "q".to_string(),
            item_id: "i".to_string(),
            data: HashMap::new(),
            pushed_at_epoch_ms: 0,
            namespace: String::new(),
        }
        .name(),
        "queue:pushed"
    );
    assert_eq!(
        Event::QueueTaken {
            queue_name: "q".to_string(),
            item_id: "i".to_string(),
            worker_name: "w".to_string(),
            namespace: String::new(),
        }
        .name(),
        "queue:taken"
    );
    assert_eq!(
        Event::QueueCompleted {
            queue_name: "q".to_string(),
            item_id: "i".to_string(),
            namespace: String::new(),
        }
        .name(),
        "queue:completed"
    );
    assert_eq!(
        Event::QueueFailed {
            queue_name: "q".to_string(),
            item_id: "i".to_string(),
            error: "e".to_string(),
            namespace: String::new(),
        }
        .name(),
        "queue:failed"
    );
    assert_eq!(
        Event::QueueItemRetry {
            queue_name: "q".to_string(),
            item_id: "i".to_string(),
            namespace: String::new(),
        }
        .name(),
        "queue:item_retry"
    );
    assert_eq!(
        Event::QueueItemDead {
            queue_name: "q".to_string(),
            item_id: "i".to_string(),
            namespace: String::new(),
        }
        .name(),
        "queue:item_dead"
    );
}

#[test]
fn event_queue_item_retry_roundtrip() {
    let event = Event::QueueItemRetry {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        namespace: "myns".to_string(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).expect("serialize");
    assert_eq!(json["type"], "queue:item_retry");
    assert_eq!(json["queue_name"], "bugs");
    assert_eq!(json["item_id"], "item-1");
    assert_eq!(json["namespace"], "myns");

    let json_str = serde_json::to_string(&event).expect("serialize");
    let parsed: Event = serde_json::from_str(&json_str).expect("deserialize");
    assert_eq!(event, parsed);
}

#[test]
fn event_queue_item_dead_roundtrip() {
    let event = Event::QueueItemDead {
        queue_name: "bugs".to_string(),
        item_id: "item-1".to_string(),
        namespace: String::new(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).expect("serialize");
    assert_eq!(json["type"], "queue:item_dead");
    assert_eq!(json["queue_name"], "bugs");

    let json_str = serde_json::to_string(&event).expect("serialize");
    let parsed: Event = serde_json::from_str(&json_str).expect("deserialize");
    assert_eq!(event, parsed);
}

// =============================================================================
// WorkerTakeComplete Event Tests
// =============================================================================

#[test]
fn event_worker_take_complete_roundtrip() {
    let event = Event::WorkerTakeComplete {
        worker_name: "fixer".to_string(),
        item_id: "item-1".to_string(),
        item: serde_json::json!({"id": "item-1", "title": "Fix bug"}),
        exit_code: 0,
        stderr: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "worker:take_complete");
    assert_eq!(json["worker_name"], "fixer");
    assert_eq!(json["item_id"], "item-1");
    assert_eq!(json["item"]["id"], "item-1");
    assert_eq!(json["exit_code"], 0);

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_worker_take_complete_failure_roundtrip() {
    let event = Event::WorkerTakeComplete {
        worker_name: "fixer".to_string(),
        item_id: "item-1".to_string(),
        item: serde_json::json!({"id": "item-1"}),
        exit_code: 1,
        stderr: Some("take command failed".to_string()),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "worker:take_complete");
    assert_eq!(json["exit_code"], 1);
    assert_eq!(json["stderr"], "take command failed");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_worker_take_name() {
    assert_eq!(
        Event::WorkerTakeComplete {
            worker_name: "w".to_string(),
            item_id: "i".to_string(),
            item: serde_json::json!({}),
            exit_code: 0,
            stderr: None,
        }
        .name(),
        "worker:take_complete"
    );
}

// =============================================================================
// AgentIdle / AgentPrompt Event Tests
// =============================================================================

#[test]
fn event_agent_idle_roundtrip() {
    let event = Event::AgentIdle {
        agent_id: AgentId::new("hook-agent-1"),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:idle");
    assert_eq!(json["agent_id"], "hook-agent-1");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_agent_prompt_roundtrip() {
    use super::PromptType;
    let event = Event::AgentPrompt {
        agent_id: AgentId::new("hook-agent-2"),
        prompt_type: PromptType::Permission,
        question_data: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:prompt");
    assert_eq!(json["agent_id"], "hook-agent-2");
    assert_eq!(json["prompt_type"], "permission");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_agent_prompt_all_types_roundtrip() {
    use super::PromptType;
    let types = vec![
        PromptType::Permission,
        PromptType::Idle,
        PromptType::PlanApproval,
        PromptType::Question,
        PromptType::Other,
    ];
    for pt in types {
        let event = Event::AgentPrompt {
            agent_id: AgentId::new("a1"),
            prompt_type: pt,
            question_data: None,
        };
        let json_str = serde_json::to_string(&event).unwrap();
        let parsed: Event = serde_json::from_str(&json_str).unwrap();
        assert_eq!(event, parsed);
    }
}

#[test]
fn event_agent_prompt_default_type() {
    // When prompt_type is missing, should default to Other
    let json = r#"{"type":"agent:prompt","agent_id":"a1"}"#;
    let parsed: Event = serde_json::from_str(json).unwrap();
    if let Event::AgentPrompt { prompt_type, .. } = &parsed {
        assert_eq!(prompt_type, &super::PromptType::Other);
    } else {
        panic!("expected AgentPrompt");
    }
}

#[test]
fn event_agent_idle_name() {
    let event = Event::AgentIdle {
        agent_id: AgentId::new("a1"),
    };
    assert_eq!(event.name(), "agent:idle");
}

#[test]
fn event_agent_prompt_name() {
    let event = Event::AgentPrompt {
        agent_id: AgentId::new("a1"),
        prompt_type: super::PromptType::Permission,
        question_data: None,
    };
    assert_eq!(event.name(), "agent:prompt");
}

// =============================================================================
// Decision Event Tests
// =============================================================================

#[test]
fn event_decision_created_roundtrip() {
    use super::{DecisionOption, DecisionSource};

    let event = Event::DecisionCreated {
        id: "dec-abc123".to_string(),
        job_id: JobId::new("pipe-1"),
        agent_id: Some("agent-1".to_string()),
        owner: None,
        source: DecisionSource::Gate,
        context: "Gate check failed".to_string(),
        options: vec![
            DecisionOption {
                label: "Approve".to_string(),
                description: None,
                recommended: true,
            },
            DecisionOption {
                label: "Reject".to_string(),
                description: Some("Stop job".to_string()),
                recommended: false,
            },
        ],
        created_at_ms: 2_000_000,
        namespace: "myns".to_string(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "decision:created");
    assert_eq!(json["id"], "dec-abc123");
    assert_eq!(json["job_id"], "pipe-1");
    assert_eq!(json["agent_id"], "agent-1");
    assert_eq!(json["source"], "gate");
    assert_eq!(json["options"].as_array().unwrap().len(), 2);

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_decision_resolved_roundtrip() {
    let event = Event::DecisionResolved {
        id: "dec-abc123".to_string(),
        chosen: Some(1),
        message: Some("Approved".to_string()),
        resolved_at_ms: 3_000_000,
        namespace: "myns".to_string(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "decision:resolved");
    assert_eq!(json["id"], "dec-abc123");
    assert_eq!(json["chosen"], 1);
    assert_eq!(json["message"], "Approved");
    assert_eq!(json["resolved_at_ms"], 3_000_000);

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_decision_resolved_freeform_only_roundtrip() {
    let event = Event::DecisionResolved {
        id: "dec-xyz".to_string(),
        chosen: None,
        message: Some("Custom response".to_string()),
        resolved_at_ms: 4_000_000,
        namespace: String::new(),
    };
    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_decision_name() {
    use super::DecisionSource;

    assert_eq!(
        Event::DecisionCreated {
            id: "d".to_string(),
            job_id: JobId::new("p"),
            agent_id: None,
            owner: None,
            source: DecisionSource::Question,
            context: "ctx".to_string(),
            options: vec![],
            created_at_ms: 0,
            namespace: String::new(),
        }
        .name(),
        "decision:created"
    );
    assert_eq!(
        Event::DecisionResolved {
            id: "d".to_string(),
            chosen: None,
            message: None,
            resolved_at_ms: 0,
            namespace: String::new(),
        }
        .name(),
        "decision:resolved"
    );
}

// =============================================================================
// log_summary() Tests
// =============================================================================

#[test]
fn log_summary_agent_state_events() {
    let cases = vec![
        (
            Event::AgentWorking {
                agent_id: AgentId::new("a1"),
                owner: None,
            },
            "agent:working agent=a1",
        ),
        (
            Event::AgentWaiting {
                agent_id: AgentId::new("a2"),
                owner: None,
            },
            "agent:waiting agent=a2",
        ),
        (
            Event::AgentFailed {
                agent_id: AgentId::new("a3"),
                error: AgentError::RateLimited,
                owner: None,
            },
            "agent:failed agent=a3",
        ),
        (
            Event::AgentExited {
                agent_id: AgentId::new("a4"),
                exit_code: Some(0),
                owner: None,
            },
            "agent:exited agent=a4",
        ),
        (
            Event::AgentGone {
                agent_id: AgentId::new("a5"),
                owner: None,
            },
            "agent:gone agent=a5",
        ),
    ];
    for (event, expected) in cases {
        assert_eq!(event.log_summary(), expected, "failed for {:?}", event);
    }
}

#[test]
fn log_summary_agent_input() {
    let event = Event::AgentInput {
        agent_id: AgentId::new("a1"),
        input: "hello".to_string(),
    };
    assert_eq!(event.log_summary(), "agent:input agent=a1");
}

#[test]
fn log_summary_agent_signal() {
    let event = Event::AgentSignal {
        agent_id: AgentId::new("a1"),
        kind: super::AgentSignalKind::Complete,
        message: Some("done".to_string()),
    };
    assert_eq!(event.log_summary(), "agent:signal id=a1 kind=Complete");

    let event = Event::AgentSignal {
        agent_id: AgentId::new("a2"),
        kind: super::AgentSignalKind::Escalate,
        message: None,
    };
    assert_eq!(event.log_summary(), "agent:signal id=a2 kind=Escalate");
}

#[test]
fn log_summary_agent_idle() {
    let event = Event::AgentIdle {
        agent_id: AgentId::new("a1"),
    };
    assert_eq!(event.log_summary(), "agent:idle agent=a1");
}

#[test]
fn log_summary_agent_stop() {
    let event = Event::AgentStop {
        agent_id: AgentId::new("a1"),
    };
    assert_eq!(event.log_summary(), "agent:stop agent=a1");
}

#[test]
fn log_summary_agent_prompt() {
    let event = Event::AgentPrompt {
        agent_id: AgentId::new("a1"),
        prompt_type: super::PromptType::Permission,
        question_data: None,
    };
    assert_eq!(
        event.log_summary(),
        "agent:prompt agent=a1 prompt_type=Permission"
    );
}

#[test]
fn log_summary_command_run_no_namespace() {
    let event = Event::CommandRun {
        job_id: JobId::new("j1"),
        job_name: "build".to_string(),
        project_root: PathBuf::from("/proj"),
        invoke_dir: PathBuf::from("/proj"),
        command: "build".to_string(),
        args: HashMap::new(),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "command:run id=j1 cmd=build");
}

#[test]
fn log_summary_command_run_with_namespace() {
    let event = Event::CommandRun {
        job_id: JobId::new("j1"),
        job_name: "build".to_string(),
        project_root: PathBuf::from("/proj"),
        invoke_dir: PathBuf::from("/proj"),
        command: "deploy".to_string(),
        args: HashMap::new(),
        namespace: "myns".to_string(),
    };
    assert_eq!(event.log_summary(), "command:run id=j1 ns=myns cmd=deploy");
}

#[test]
fn log_summary_job_created_no_namespace() {
    let event = Event::JobCreated {
        id: JobId::new("j1"),
        kind: "build".to_string(),
        name: "test".to_string(),
        runbook_hash: "abc".to_string(),
        cwd: PathBuf::from("/"),
        vars: HashMap::new(),
        initial_step: "init".to_string(),
        created_at_epoch_ms: 0,
        namespace: String::new(),
        cron_name: None,
    };
    assert_eq!(
        event.log_summary(),
        "job:created id=j1 kind=build name=test"
    );
}

#[test]
fn log_summary_job_created_with_namespace() {
    let event = Event::JobCreated {
        id: JobId::new("j1"),
        kind: "build".to_string(),
        name: "test".to_string(),
        runbook_hash: "abc".to_string(),
        cwd: PathBuf::from("/"),
        vars: HashMap::new(),
        initial_step: "init".to_string(),
        created_at_epoch_ms: 0,
        namespace: "prod".to_string(),
        cron_name: None,
    };
    assert_eq!(
        event.log_summary(),
        "job:created id=j1 ns=prod kind=build name=test"
    );
}

#[test]
fn log_summary_job_advanced() {
    let event = Event::JobAdvanced {
        id: JobId::new("j1"),
        step: "deploy".to_string(),
    };
    assert_eq!(event.log_summary(), "job:advanced id=j1 step=deploy");
}

#[test]
fn log_summary_job_updated() {
    let event = Event::JobUpdated {
        id: JobId::new("j1"),
        vars: HashMap::new(),
    };
    assert_eq!(event.log_summary(), "job:updated id=j1");
}

#[test]
fn log_summary_job_resume() {
    let event = Event::JobResume {
        id: JobId::new("j1"),
        message: None,
        vars: HashMap::new(),
        kill: false,
    };
    assert_eq!(event.log_summary(), "job:resume id=j1");
}

#[test]
fn log_summary_job_cancelling_cancel_deleted() {
    assert_eq!(
        Event::JobCancelling {
            id: JobId::new("j1")
        }
        .log_summary(),
        "job:cancelling id=j1"
    );
    assert_eq!(
        Event::JobCancel {
            id: JobId::new("j2")
        }
        .log_summary(),
        "job:cancel id=j2"
    );
    assert_eq!(
        Event::JobDeleted {
            id: JobId::new("j3")
        }
        .log_summary(),
        "job:deleted id=j3"
    );
}

#[test]
fn log_summary_runbook_loaded() {
    let runbook = serde_json::json!({
        "agents": {"builder": {}, "tester": {}},
        "jobs": {"ci": {}}
    });
    let event = Event::RunbookLoaded {
        hash: "abcdef1234567890".to_string(),
        version: 3,
        runbook,
    };
    assert_eq!(
        event.log_summary(),
        "runbook:loaded hash=abcdef123456 v=3 agents=2 jobs=1"
    );
}

#[test]
fn log_summary_runbook_loaded_empty() {
    let runbook = serde_json::json!({});
    let event = Event::RunbookLoaded {
        hash: "short".to_string(),
        version: 1,
        runbook,
    };
    assert_eq!(
        event.log_summary(),
        "runbook:loaded hash=short v=1 agents=0 jobs=0"
    );
}

#[test]
fn log_summary_session_created_job_owner() {
    use crate::owner::OwnerId;
    let event = Event::SessionCreated {
        id: SessionId::new("s1"),
        owner: OwnerId::Job(JobId::new("j1")),
    };
    assert_eq!(event.log_summary(), "session:created id=s1 job=j1");
}

#[test]
fn log_summary_session_created_agent_run_owner() {
    use crate::agent_run::AgentRunId;
    use crate::owner::OwnerId;
    let event = Event::SessionCreated {
        id: SessionId::new("s1"),
        owner: OwnerId::AgentRun(AgentRunId::new("ar1")),
    };
    assert_eq!(event.log_summary(), "session:created id=s1 agent_run=ar1");
}

#[test]
fn log_summary_session_input_deleted() {
    assert_eq!(
        Event::SessionInput {
            id: SessionId::new("s1"),
            input: "text".to_string(),
        }
        .log_summary(),
        "session:input id=s1"
    );
    assert_eq!(
        Event::SessionDeleted {
            id: SessionId::new("s2"),
        }
        .log_summary(),
        "session:deleted id=s2"
    );
}

#[test]
fn log_summary_shell_exited() {
    let event = Event::ShellExited {
        job_id: JobId::new("j1"),
        step: "init".to_string(),
        exit_code: 42,
        stdout: None,
        stderr: None,
    };
    assert_eq!(event.log_summary(), "shell:exited job=j1 step=init exit=42");
}

#[test]
fn log_summary_step_events() {
    assert_eq!(
        Event::StepStarted {
            job_id: JobId::new("j1"),
            step: "build".to_string(),
            agent_id: None,
            agent_name: None,
        }
        .log_summary(),
        "step:started job=j1 step=build"
    );
    assert_eq!(
        Event::StepWaiting {
            job_id: JobId::new("j1"),
            step: "review".to_string(),
            reason: Some("gate failed".to_string()),
            decision_id: None,
        }
        .log_summary(),
        "step:waiting job=j1 step=review"
    );
    assert_eq!(
        Event::StepCompleted {
            job_id: JobId::new("j1"),
            step: "deploy".to_string(),
        }
        .log_summary(),
        "step:completed job=j1 step=deploy"
    );
    assert_eq!(
        Event::StepFailed {
            job_id: JobId::new("j1"),
            step: "test".to_string(),
            error: "oops".to_string(),
        }
        .log_summary(),
        "step:failed job=j1 step=test"
    );
}

#[test]
fn log_summary_shutdown_and_custom() {
    assert_eq!(Event::Shutdown.log_summary(), "system:shutdown");
    assert_eq!(Event::Custom.log_summary(), "custom");
}

#[test]
fn log_summary_timer_start() {
    let event = Event::TimerStart {
        id: TimerId::new("t1"),
    };
    assert_eq!(event.log_summary(), "timer:start id=t1");
}

#[test]
fn log_summary_workspace_events() {
    assert_eq!(
        Event::WorkspaceCreated {
            id: WorkspaceId::new("ws1"),
            path: PathBuf::from("/tmp/ws"),
            branch: Some("main".to_string()),
            owner: None,
            workspace_type: None,
        }
        .log_summary(),
        "workspace:created id=ws1"
    );
    assert_eq!(
        Event::WorkspaceReady {
            id: WorkspaceId::new("ws1"),
        }
        .log_summary(),
        "workspace:ready id=ws1"
    );
    assert_eq!(
        Event::WorkspaceFailed {
            id: WorkspaceId::new("ws1"),
            reason: "disk full".to_string(),
        }
        .log_summary(),
        "workspace:failed id=ws1"
    );
    assert_eq!(
        Event::WorkspaceDeleted {
            id: WorkspaceId::new("ws1"),
        }
        .log_summary(),
        "workspace:deleted id=ws1"
    );
    assert_eq!(
        Event::WorkspaceDrop {
            id: WorkspaceId::new("ws1"),
        }
        .log_summary(),
        "workspace:drop id=ws1"
    );
}

#[test]
fn log_summary_cron_started_stopped() {
    assert_eq!(
        Event::CronStarted {
            cron_name: "nightly".to_string(),
            project_root: PathBuf::from("/proj"),
            runbook_hash: "abc".to_string(),
            interval: "1h".to_string(),
            run_target: "job:build".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "cron:started cron=nightly"
    );
    assert_eq!(
        Event::CronStopped {
            cron_name: "nightly".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "cron:stopped cron=nightly"
    );
}

#[test]
fn log_summary_cron_once_job_target() {
    let event = Event::CronOnce {
        cron_name: "nightly".to_string(),
        job_id: JobId::new("j1"),
        job_name: "build".to_string(),
        job_kind: "build".to_string(),
        agent_run_id: None,
        agent_name: None,
        project_root: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        run_target: "job:build".to_string(),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:once cron=nightly job=j1");
}

#[test]
fn log_summary_cron_once_agent_target() {
    let event = Event::CronOnce {
        cron_name: "nightly".to_string(),
        job_id: JobId::default(),
        job_name: String::new(),
        job_kind: String::new(),
        agent_run_id: Some("ar1".to_string()),
        agent_name: Some("builder".to_string()),
        project_root: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        run_target: "agent:builder".to_string(),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:once cron=nightly agent=builder");
}

#[test]
fn log_summary_cron_fired_job() {
    let event = Event::CronFired {
        cron_name: "nightly".to_string(),
        job_id: JobId::new("j1"),
        agent_run_id: None,
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:fired cron=nightly job=j1");
}

#[test]
fn log_summary_cron_fired_agent_run() {
    let event = Event::CronFired {
        cron_name: "nightly".to_string(),
        job_id: JobId::default(),
        agent_run_id: Some("ar1".to_string()),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:fired cron=nightly agent_run=ar1");
}

#[test]
fn log_summary_cron_deleted_no_namespace() {
    let event = Event::CronDeleted {
        cron_name: "nightly".to_string(),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "cron:deleted cron=nightly");
}

#[test]
fn log_summary_cron_deleted_with_namespace() {
    let event = Event::CronDeleted {
        cron_name: "nightly".to_string(),
        namespace: "prod".to_string(),
    };
    assert_eq!(event.log_summary(), "cron:deleted cron=nightly ns=prod");
}

#[test]
fn log_summary_worker_events() {
    assert_eq!(
        Event::WorkerStarted {
            worker_name: "fixer".to_string(),
            project_root: PathBuf::from("/proj"),
            runbook_hash: "abc".to_string(),
            queue_name: "bugs".to_string(),
            concurrency: 2,
            namespace: String::new(),
        }
        .log_summary(),
        "worker:started worker=fixer"
    );
    assert_eq!(
        Event::WorkerWake {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "worker:wake worker=fixer"
    );
    assert_eq!(
        Event::WorkerStopped {
            worker_name: "fixer".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "worker:stopped worker=fixer"
    );
}

#[test]
fn log_summary_worker_poll_complete() {
    let event = Event::WorkerPollComplete {
        worker_name: "fixer".to_string(),
        items: vec![
            serde_json::json!({"id": "1"}),
            serde_json::json!({"id": "2"}),
        ],
    };
    assert_eq!(
        event.log_summary(),
        "worker:poll_complete worker=fixer items=2"
    );
}

#[test]
fn log_summary_worker_take_complete() {
    let event = Event::WorkerTakeComplete {
        worker_name: "fixer".to_string(),
        item_id: "item-1".to_string(),
        item: serde_json::json!({}),
        exit_code: 0,
        stderr: None,
    };
    assert_eq!(
        event.log_summary(),
        "worker:take_complete worker=fixer item=item-1 exit=0"
    );
}

#[test]
fn log_summary_worker_item_dispatched() {
    let event = Event::WorkerItemDispatched {
        worker_name: "fixer".to_string(),
        item_id: "item-1".to_string(),
        job_id: JobId::new("j1"),
        namespace: String::new(),
    };
    assert_eq!(
        event.log_summary(),
        "worker:item_dispatched worker=fixer item=item-1 job=j1"
    );
}

#[test]
fn log_summary_worker_resized_no_namespace() {
    let event = Event::WorkerResized {
        worker_name: "fixer".to_string(),
        concurrency: 4,
        namespace: String::new(),
    };
    assert_eq!(
        event.log_summary(),
        "worker:resized worker=fixer concurrency=4"
    );
}

#[test]
fn log_summary_worker_resized_with_namespace() {
    let event = Event::WorkerResized {
        worker_name: "fixer".to_string(),
        concurrency: 4,
        namespace: "prod".to_string(),
    };
    assert_eq!(
        event.log_summary(),
        "worker:resized worker=fixer ns=prod concurrency=4"
    );
}

#[test]
fn log_summary_worker_deleted_no_namespace() {
    let event = Event::WorkerDeleted {
        worker_name: "fixer".to_string(),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "worker:deleted worker=fixer");
}

#[test]
fn log_summary_worker_deleted_with_namespace() {
    let event = Event::WorkerDeleted {
        worker_name: "fixer".to_string(),
        namespace: "prod".to_string(),
    };
    assert_eq!(event.log_summary(), "worker:deleted worker=fixer ns=prod");
}

#[test]
fn log_summary_queue_events() {
    assert_eq!(
        Event::QueuePushed {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            data: HashMap::new(),
            pushed_at_epoch_ms: 0,
            namespace: String::new(),
        }
        .log_summary(),
        "queue:pushed queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueTaken {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            worker_name: "w".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:taken queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueCompleted {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:completed queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueFailed {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            error: "e".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:failed queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueDropped {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:dropped queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueItemRetry {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:item_retry queue=bugs item=i1"
    );
    assert_eq!(
        Event::QueueItemDead {
            queue_name: "bugs".to_string(),
            item_id: "i1".to_string(),
            namespace: String::new(),
        }
        .log_summary(),
        "queue:item_dead queue=bugs item=i1"
    );
}

#[test]
fn log_summary_decision_created_job_owner() {
    use super::DecisionSource;
    let event = Event::DecisionCreated {
        id: "d1".to_string(),
        job_id: JobId::new("j1"),
        agent_id: None,
        owner: None,
        source: DecisionSource::Gate,
        context: "ctx".to_string(),
        options: vec![],
        created_at_ms: 0,
        namespace: String::new(),
    };
    assert_eq!(
        event.log_summary(),
        "decision:created id=d1 job=j1 source=Gate"
    );
}

#[test]
fn log_summary_decision_created_agent_run_owner() {
    use super::DecisionSource;
    use crate::agent_run::AgentRunId;
    use crate::owner::OwnerId;
    let event = Event::DecisionCreated {
        id: "d1".to_string(),
        job_id: JobId::default(),
        agent_id: None,
        owner: Some(OwnerId::AgentRun(AgentRunId::new("ar1"))),
        source: DecisionSource::Question,
        context: "ctx".to_string(),
        options: vec![],
        created_at_ms: 0,
        namespace: String::new(),
    };
    assert_eq!(
        event.log_summary(),
        "decision:created id=d1 agent_run=ar1 source=Question"
    );
}

#[test]
fn log_summary_decision_resolved_with_chosen() {
    let event = Event::DecisionResolved {
        id: "d1".to_string(),
        chosen: Some(2),
        message: None,
        resolved_at_ms: 0,
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "decision:resolved id=d1 chosen=2");
}

#[test]
fn log_summary_decision_resolved_no_chosen() {
    let event = Event::DecisionResolved {
        id: "d1".to_string(),
        chosen: None,
        message: Some("custom".to_string()),
        resolved_at_ms: 0,
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "decision:resolved id=d1");
}

#[test]
fn log_summary_agent_run_created_no_namespace() {
    use crate::agent_run::AgentRunId;
    let event = Event::AgentRunCreated {
        id: AgentRunId::new("ar1"),
        agent_name: "builder".to_string(),
        command_name: "build".to_string(),
        namespace: String::new(),
        cwd: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        vars: HashMap::new(),
        created_at_epoch_ms: 0,
    };
    assert_eq!(
        event.log_summary(),
        "agent_run:created id=ar1 agent=builder"
    );
}

#[test]
fn log_summary_agent_run_created_with_namespace() {
    use crate::agent_run::AgentRunId;
    let event = Event::AgentRunCreated {
        id: AgentRunId::new("ar1"),
        agent_name: "builder".to_string(),
        command_name: "build".to_string(),
        namespace: "prod".to_string(),
        cwd: PathBuf::from("/proj"),
        runbook_hash: "abc".to_string(),
        vars: HashMap::new(),
        created_at_epoch_ms: 0,
    };
    assert_eq!(
        event.log_summary(),
        "agent_run:created id=ar1 ns=prod agent=builder"
    );
}

#[test]
fn log_summary_agent_run_started() {
    use crate::agent_run::AgentRunId;
    let event = Event::AgentRunStarted {
        id: AgentRunId::new("ar1"),
        agent_id: AgentId::new("a1"),
    };
    assert_eq!(event.log_summary(), "agent_run:started id=ar1 agent_id=a1");
}

#[test]
fn log_summary_agent_run_status_changed_with_reason() {
    use crate::agent_run::{AgentRunId, AgentRunStatus};
    let event = Event::AgentRunStatusChanged {
        id: AgentRunId::new("ar1"),
        status: AgentRunStatus::Failed,
        reason: Some("timeout".to_string()),
    };
    assert_eq!(
        event.log_summary(),
        "agent_run:status_changed id=ar1 status=failed reason=timeout"
    );
}

#[test]
fn log_summary_agent_run_status_changed_no_reason() {
    use crate::agent_run::{AgentRunId, AgentRunStatus};
    let event = Event::AgentRunStatusChanged {
        id: AgentRunId::new("ar1"),
        status: AgentRunStatus::Running,
        reason: None,
    };
    assert_eq!(
        event.log_summary(),
        "agent_run:status_changed id=ar1 status=running"
    );
}

#[test]
fn log_summary_agent_run_deleted() {
    use crate::agent_run::AgentRunId;
    let event = Event::AgentRunDeleted {
        id: AgentRunId::new("ar1"),
    };
    assert_eq!(event.log_summary(), "agent_run:deleted id=ar1");
}
