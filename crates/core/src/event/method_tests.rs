// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for `Event` methods: `job_id`, `name`, `from_agent_state`,
//! `as_agent_state`.

use crate::crew::CrewId;
use crate::event::*;
use crate::owner::OwnerId;
use crate::AgentState;

#[test]
fn event_job_id_returns_id_for_job_events() {
    let cases: Vec<(Event, JobId)> = vec![
        (
            Event::CommandRun {
                owner: OwnerId::Job(JobId::from_string("p1")),
                name: "b".to_string(),
                project_path: PathBuf::from("/"),
                invoke_dir: PathBuf::from("/"),
                command: "build".to_string(),
                args: HashMap::new(),
                project: String::new(),
            },
            JobId::from_string("p1"),
        ),
        (
            Event::JobCreated {
                id: JobId::from_string("p6"),
                kind: "build".to_string(),
                name: "test".to_string(),
                runbook_hash: "abc".to_string(),
                cwd: PathBuf::from("/"),
                vars: HashMap::new(),
                initial_step: "init".to_string(),
                created_at_ms: 1_000_000,
                project: String::new(),
                cron: None,
            },
            JobId::from_string("p6"),
        ),
    ];

    for (event, expected_id) in cases {
        assert_eq!(event.job_id(), Some(&expected_id), "wrong job_id for {:?}", event);
    }
}

#[test]
fn event_job_id_returns_none_for_non_job_events() {
    let events = vec![
        Event::TimerStart { id: TimerId::from_string("t") },
        Event::AgentSpawned {
            id: AgentId::from_string("a1"),
            owner: CrewId::from_string("r1").into(),
            runtime: Default::default(),
            auth_token: None,
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
    let agent_id = AgentId::from_string("test");

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
    let agent_id = AgentId::from_string("test");

    let event = Event::AgentWorking { id: agent_id.clone(), owner: OwnerId::Job(JobId::default()) };
    let (id, state, _owner) = event.as_agent_state().unwrap();
    assert_eq!(id, &agent_id);
    assert!(matches!(state, AgentState::Working));

    let event = Event::Shutdown;
    assert!(event.as_agent_state().is_none());
}

#[test]
fn event_queue_name_returns_correct_strings() {
    assert_eq!(
        Event::QueuePushed {
            queue: "q".to_string(),
            item_id: "i".to_string(),
            data: HashMap::new(),
            pushed_at_ms: 0,
            project: String::new(),
        }
        .name(),
        "queue:pushed"
    );
    assert_eq!(
        Event::QueueTaken {
            queue: "q".to_string(),
            item_id: "i".to_string(),
            worker: "w".to_string(),
            project: String::new(),
        }
        .name(),
        "queue:taken"
    );
    assert_eq!(
        Event::QueueCompleted {
            queue: "q".to_string(),
            item_id: "i".to_string(),
            project: String::new(),
        }
        .name(),
        "queue:completed"
    );
    assert_eq!(
        Event::QueueFailed {
            queue: "q".to_string(),
            item_id: "i".to_string(),
            error: "e".to_string(),
            project: String::new(),
        }
        .name(),
        "queue:failed"
    );
    assert_eq!(
        Event::QueueRetry {
            queue: "q".to_string(),
            item_id: "i".to_string(),
            project: String::new(),
        }
        .name(),
        "queue:retry"
    );
    assert_eq!(
        Event::QueueDead {
            queue: "q".to_string(),
            item_id: "i".to_string(),
            project: String::new(),
        }
        .name(),
        "queue:dead"
    );
}

#[test]
fn event_worker_take_name() {
    assert_eq!(
        Event::WorkerTook {
            worker: "w".to_string(),
            project: String::new(),
            item_id: "i".to_string(),
            item: serde_json::json!({}),
            exit_code: 0,
            stderr: None,
        }
        .name(),
        "worker:took"
    );
}

#[test]
fn event_agent_idle_name() {
    let event = Event::AgentIdle { id: AgentId::from_string("a1") };
    assert_eq!(event.name(), "agent:idle");
}

#[test]
fn event_agent_prompt_name() {
    let event = Event::AgentPrompt {
        id: AgentId::from_string("a1"),
        prompt_type: PromptType::Permission,
        questions: None,
        last_message: None,
    };
    assert_eq!(event.name(), "agent:prompt");
}

#[test]
fn event_decision_name() {
    assert_eq!(
        Event::DecisionCreated {
            id: DecisionId::from_string("d"),
            agent_id: AgentId::from_string("a"),
            owner: JobId::from_string("p").into(),
            source: DecisionSource::Question,
            context: "ctx".to_string(),
            options: vec![],
            questions: None,
            created_at_ms: 0,
            project: String::new(),
        }
        .name(),
        "decision:created"
    );
    assert_eq!(
        Event::DecisionResolved {
            id: DecisionId::from_string("d"),
            choices: vec![],
            message: None,
            resolved_at_ms: 0,
            project: String::new(),
        }
        .name(),
        "decision:resolved"
    );
}

#[test]
fn event_crew_resume_name() {
    assert_eq!(
        Event::CrewResume { id: CrewId::from_string("run-1"), message: None, kill: false }.name(),
        "crew:resume"
    );
}
