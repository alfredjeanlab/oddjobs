// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Serialization roundtrip tests for core event variants (agent state, job,
//! step, workspace, shell) and tests for `Event` methods (`job_id`,
//! `from_agent_state`, `as_agent_state`).

use super::*;

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
            owner: OwnerId::Job(JobId::new("pipe-1")),
        },
        Event::AgentFailed {
            agent_id: AgentId::new("agent-2"),
            error: AgentError::RateLimited,
            owner: OwnerId::Job(JobId::new("pipe-1")),
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
        owner: OwnerId::Job(JobId::default()),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:working");
    assert_eq!(json["agent_id"], "a1");

    assert_roundtrip(&event);
}

#[test]
fn event_agent_failed_roundtrip() {
    let event = Event::AgentFailed {
        agent_id: AgentId::new("a2"),
        error: AgentError::RateLimited,
        owner: OwnerId::Job(JobId::default()),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:failed");
    assert_eq!(json["agent_id"], "a2");

    assert_roundtrip(&event);
}

#[test]
fn event_agent_exited_roundtrip() {
    let event = Event::AgentExited {
        agent_id: AgentId::new("a3"),
        exit_code: Some(42),
        owner: OwnerId::Job(JobId::default()),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:exited");
    assert_eq!(json["exit_code"], 42);

    assert_roundtrip(&event);
}

#[test]
fn event_agent_exited_no_code_roundtrip() {
    let event = Event::AgentExited {
        agent_id: AgentId::new("a4"),
        exit_code: None,
        owner: OwnerId::Job(JobId::default()),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "agent:exited");
    assert!(json["exit_code"].is_null());

    assert_roundtrip(&event);
}

#[test]
fn event_agent_gone_roundtrip() {
    let event = Event::AgentGone {
        agent_id: AgentId::new("a5"),
        owner: OwnerId::Job(JobId::default()),
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
}

#[test]
fn event_workspace_drop_roundtrip() {
    let event = Event::WorkspaceDrop {
        id: WorkspaceId::new("ws-1"),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "workspace:drop");
    assert_eq!(json["id"], "ws-1");

    assert_roundtrip(&event);
}

#[test]
fn event_workspace_ready_roundtrip() {
    let event = Event::WorkspaceReady {
        id: WorkspaceId::new("ws-1"),
    };
    let json: serde_json::Value = serde_json::to_value(&event).unwrap();
    assert_eq!(json["type"], "workspace:ready");
    assert_eq!(json["id"], "ws-1");

    assert_roundtrip(&event);
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

    assert_roundtrip(&event);
}

// =============================================================================
// Event method tests
// =============================================================================

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
        Event::from_agent_state(
            agent_id.clone(),
            AgentState::Working,
            OwnerId::Job(JobId::default())
        ),
        Event::AgentWorking { .. }
    ));
    assert!(matches!(
        Event::from_agent_state(
            agent_id.clone(),
            AgentState::WaitingForInput,
            OwnerId::Job(JobId::default())
        ),
        Event::AgentWaiting { .. }
    ));
    assert!(matches!(
        Event::from_agent_state(
            agent_id.clone(),
            AgentState::SessionGone,
            OwnerId::Job(JobId::default())
        ),
        Event::AgentGone { .. }
    ));
}

#[test]
fn event_as_agent_state() {
    let agent_id = AgentId::new("test");

    let event = Event::AgentWorking {
        agent_id: agent_id.clone(),
        owner: OwnerId::Job(JobId::default()),
    };
    let (id, state, _owner) = event.as_agent_state().unwrap();
    assert_eq!(id, &agent_id);
    assert!(matches!(state, AgentState::Working));

    let event = Event::Shutdown;
    assert!(event.as_agent_state().is_none());
}
