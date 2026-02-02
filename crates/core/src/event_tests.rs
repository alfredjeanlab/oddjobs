// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::agent::AgentError;

#[test]
fn event_serialization_roundtrip() {
    let events = vec![
        Event::CommandRun {
            pipeline_id: PipelineId::new("pipe-1"),
            pipeline_name: "build".to_string(),
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
        },
        Event::AgentFailed {
            agent_id: AgentId::new("agent-2"),
            error: AgentError::RateLimited,
        },
        Event::ShellExited {
            pipeline_id: PipelineId::new("pipe-1"),
            step: "init".to_string(),
            exit_code: 0,
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
    };
    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
    let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(json["type"], "agent:gone");
}

#[test]
fn event_pipeline_created_roundtrip() {
    let event = Event::PipelineCreated {
        id: PipelineId::new("pipe-1"),
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
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "pipeline:created");
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
        pipeline_id: PipelineId::new("pipe-1"),
        step: "build".to_string(),
        agent_id: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "step:started");
    assert_eq!(json["pipeline_id"], "pipe-1");
    assert_eq!(json["step"], "build");
    assert!(json.get("agent_id").is_none());

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_step_waiting_roundtrip() {
    let event = Event::StepWaiting {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "review".to_string(),
        reason: Some("gate failed".to_string()),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "step:waiting");
    assert_eq!(json["pipeline_id"], "pipe-1");
    assert_eq!(json["step"], "review");
    assert_eq!(json["reason"], "gate failed");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_step_completed_roundtrip() {
    let event = Event::StepCompleted {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "deploy".to_string(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "step:completed");
    assert_eq!(json["pipeline_id"], "pipe-1");
    assert_eq!(json["step"], "deploy");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_step_failed_roundtrip() {
    let event = Event::StepFailed {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "test".to_string(),
        error: "something went wrong".to_string(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "step:failed");
    assert_eq!(json["pipeline_id"], "pipe-1");
    assert_eq!(json["step"], "test");
    assert_eq!(json["error"], "something went wrong");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_pipeline_resume_roundtrip() {
    let event = Event::PipelineResume {
        id: PipelineId::new("pipe-1"),
        message: Some("try again".to_string()),
        vars: [("key".to_string(), "value".to_string())]
            .into_iter()
            .collect(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "pipeline:resume");
    assert_eq!(json["id"], "pipe-1");
    assert_eq!(json["message"], "try again");

    let json_str = serde_json::to_string(&event).unwrap();
    let parsed: Event = serde_json::from_str(&json_str).unwrap();
    assert_eq!(event, parsed);
}

#[test]
fn event_pipeline_resume_no_message_roundtrip() {
    let event = Event::PipelineResume {
        id: PipelineId::new("pipe-1"),
        message: None,
        vars: HashMap::new(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "pipeline:resume");
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
fn event_pipeline_id_returns_id_for_pipeline_events() {
    let cases: Vec<(Event, PipelineId)> = vec![
        (
            Event::CommandRun {
                pipeline_id: PipelineId::new("p1"),
                pipeline_name: "b".to_string(),
                project_root: PathBuf::from("/"),
                invoke_dir: PathBuf::from("/"),
                command: "build".to_string(),
                args: HashMap::new(),
                namespace: String::new(),
            },
            PipelineId::new("p1"),
        ),
        (
            Event::PipelineCreated {
                id: PipelineId::new("p6"),
                kind: "build".to_string(),
                name: "test".to_string(),
                runbook_hash: "abc".to_string(),
                cwd: PathBuf::from("/"),
                vars: HashMap::new(),
                initial_step: "init".to_string(),
                created_at_epoch_ms: 1_000_000,
                namespace: String::new(),
            },
            PipelineId::new("p6"),
        ),
    ];

    for (event, expected_id) in cases {
        assert_eq!(
            event.pipeline_id(),
            Some(&expected_id),
            "wrong pipeline_id for {:?}",
            event
        );
    }
}

#[test]
fn event_pipeline_id_returns_none_for_non_pipeline_events() {
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
        assert_eq!(event.pipeline_id(), None, "expected None for {:?}", event);
    }
}

#[test]
fn event_from_agent_state() {
    let agent_id = AgentId::new("test");

    assert!(matches!(
        Event::from_agent_state(agent_id.clone(), AgentState::Working),
        Event::AgentWorking { .. }
    ));
    assert!(matches!(
        Event::from_agent_state(agent_id.clone(), AgentState::WaitingForInput),
        Event::AgentWaiting { .. }
    ));
    assert!(matches!(
        Event::from_agent_state(agent_id.clone(), AgentState::SessionGone),
        Event::AgentGone { .. }
    ));
}

#[test]
fn event_as_agent_state() {
    let agent_id = AgentId::new("test");

    let event = Event::AgentWorking {
        agent_id: agent_id.clone(),
    };
    let (id, state) = event.as_agent_state().unwrap();
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
        error: "pipeline failed".to_string(),
        namespace: String::new(),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "queue:failed");
    assert_eq!(json["error"], "pipeline failed");

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
    };
    assert_eq!(event.name(), "agent:prompt");
}
