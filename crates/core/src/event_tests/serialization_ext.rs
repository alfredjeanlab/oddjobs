// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Serialization roundtrip and `.name()` tests for queue, worker,
//! agent idle/prompt, and decision event variants.

use super::*;

// =============================================================================
// Queue Event Tests
// =============================================================================

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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
}

#[test]
fn event_agent_prompt_roundtrip() {
    let event = Event::AgentPrompt {
        agent_id: AgentId::new("hook-agent-2"),
        prompt_type: PromptType::Permission,
        question_data: None,
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:prompt");
    assert_eq!(json["agent_id"], "hook-agent-2");
    assert_eq!(json["prompt_type"], "permission");

    assert_roundtrip(&event);
}

#[test]
fn event_agent_prompt_all_types_roundtrip() {
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
        assert_roundtrip(&event);
    }
}

#[test]
fn event_agent_prompt_default_type() {
    // When prompt_type is missing, should default to Other
    let json = r#"{"type":"agent:prompt","agent_id":"a1"}"#;
    let parsed: Event = serde_json::from_str(json).unwrap();
    if let Event::AgentPrompt { prompt_type, .. } = &parsed {
        assert_eq!(prompt_type, &PromptType::Other);
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
        prompt_type: PromptType::Permission,
        question_data: None,
    };
    assert_eq!(event.name(), "agent:prompt");
}

// =============================================================================
// Decision Event Tests
// =============================================================================

#[test]
fn event_decision_created_roundtrip() {
    let event = Event::DecisionCreated {
        id: "dec-abc123".to_string(),
        job_id: JobId::new("pipe-1"),
        agent_id: Some("agent-1".to_string()),
        owner: OwnerId::Job(JobId::new("pipe-1")),
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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
    assert_roundtrip(&event);
}

#[test]
fn event_decision_name() {
    assert_eq!(
        Event::DecisionCreated {
            id: "d".to_string(),
            job_id: JobId::new("p"),
            agent_id: None,
            owner: OwnerId::Job(JobId::new("p")),
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
