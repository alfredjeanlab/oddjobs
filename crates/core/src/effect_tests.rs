// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::{JobId, OwnerId};

#[test]
fn effect_serialization_roundtrip() {
    let effects = vec![
        Effect::Emit { event: Event::JobDeleted { id: JobId::new("job-1") } },
        Effect::SpawnAgent {
            agent_id: AgentId::new("agent-1"),
            agent_name: "claude".to_string(),
            owner: JobId::new("job-1").into(),
            workspace_path: PathBuf::from("/work"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![("KEY".to_string(), "value".to_string())],
            cwd: Some(PathBuf::from("/work")),
            unset_env: vec![],
            resume: false,
            container: None,
        },
        Effect::SendToAgent { agent_id: AgentId::new("agent-1"), input: "hello".to_string() },
        Effect::KillAgent { agent_id: AgentId::new("agent-1") },
        Effect::CreateWorkspace {
            workspace_id: crate::WorkspaceId::new("ws-1"),
            path: PathBuf::from("/work/tree"),
            owner: OwnerId::Job(crate::JobId::new("job-1")),
            workspace_type: Some("folder".to_string()),
            repo_root: None,
            branch: None,
            start_point: None,
        },
        Effect::DeleteWorkspace { workspace_id: crate::WorkspaceId::new("ws-1") },
        Effect::SetTimer { id: TimerId::new("timer-1"), duration: Duration::from_secs(60) },
        Effect::CancelTimer { id: TimerId::new("timer-1") },
        Effect::Shell {
            owner: Some(JobId::new("job-1").into()),
            step: "init".to_string(),
            command: "echo hello".to_string(),
            cwd: PathBuf::from("/tmp"),
            env: [("KEY".to_string(), "value".to_string())].into_iter().collect(),
            container: None,
        },
        Effect::PollQueue {
            worker_name: "fixer".to_string(),
            project: String::new(),
            list_command: "echo '[]'".to_string(),
            cwd: PathBuf::from("/work"),
        },
        Effect::TakeQueueItem {
            worker_name: "fixer".to_string(),
            project: String::new(),
            take_command: "echo taken".to_string(),
            cwd: PathBuf::from("/work"),
            item_id: "item-1".to_string(),
            item: serde_json::json!({"id": "item-1", "title": "test"}),
        },
        Effect::Notify { title: "Build complete".to_string(), message: "Success!".to_string() },
    ];

    for effect in effects {
        let json = serde_json::to_string(&effect).unwrap();
        let parsed: Effect = serde_json::from_str(&json).unwrap();
        assert_eq!(effect, parsed);
    }
}

#[test]
fn traced_effect_names() {
    let cases: Vec<(Effect, &str)> = vec![
        (Effect::Emit { event: Event::Shutdown }, "emit"),
        (
            Effect::SpawnAgent {
                agent_id: AgentId::new("a"),
                agent_name: "claude".to_string(),
                owner: JobId::new("p").into(),
                workspace_path: PathBuf::from("/w"),
                input: HashMap::new(),
                command: "claude".to_string(),
                env: vec![],
                cwd: None,
                unset_env: vec![],
                resume: false,
                container: None,
            },
            "spawn_agent",
        ),
        (
            Effect::SendToAgent { agent_id: AgentId::new("a"), input: "i".to_string() },
            "send_to_agent",
        ),
        (Effect::KillAgent { agent_id: AgentId::new("a") }, "kill_agent"),
        (
            Effect::CreateWorkspace {
                workspace_id: crate::WorkspaceId::new("ws"),
                path: PathBuf::from("/p"),
                owner: OwnerId::Job(crate::JobId::new("test")),
                workspace_type: None,
                repo_root: None,
                branch: None,
                start_point: None,
            },
            "create_workspace",
        ),
        (
            Effect::DeleteWorkspace { workspace_id: crate::WorkspaceId::new("ws") },
            "delete_workspace",
        ),
        (Effect::SetTimer { id: TimerId::new("t"), duration: Duration::from_secs(1) }, "set_timer"),
        (Effect::CancelTimer { id: TimerId::new("t") }, "cancel_timer"),
        (
            Effect::Shell {
                owner: Some(JobId::new("p").into()),
                step: "init".to_string(),
                command: "cmd".to_string(),
                cwd: PathBuf::from("/"),
                env: HashMap::new(),
                container: None,
            },
            "shell",
        ),
        (
            Effect::PollQueue {
                worker_name: "w".to_string(),
                project: String::new(),
                list_command: "cmd".to_string(),
                cwd: PathBuf::from("/"),
            },
            "poll_queue",
        ),
        (
            Effect::TakeQueueItem {
                worker_name: "w".to_string(),
                project: String::new(),
                take_command: "cmd".to_string(),
                cwd: PathBuf::from("/"),
                item_id: "i".to_string(),
                item: serde_json::json!({}),
            },
            "take_queue_item",
        ),
        (Effect::Notify { title: "t".to_string(), message: "m".to_string() }, "notify"),
    ];

    for (effect, expected_name) in cases {
        assert_eq!(effect.name(), expected_name);
    }
}

#[test]
fn traced_effect_fields() {
    // Test Emit fields
    let effect = Effect::Emit { event: Event::JobDeleted { id: JobId::new("job-1") } };
    let fields = effect.fields();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].0, "event");

    // Test SpawnAgent fields
    let effect = Effect::SpawnAgent {
        agent_id: AgentId::new("agent-1"),
        agent_name: "claude".to_string(),
        owner: JobId::new("job-1").into(),
        workspace_path: PathBuf::from("/work"),
        input: HashMap::new(),
        command: "claude".to_string(),
        env: vec![],
        cwd: Some(PathBuf::from("/work")),
        unset_env: vec![],
        resume: false,
        container: None,
    };
    let fields = effect.fields();
    assert_eq!(fields.len(), 6);
    assert_eq!(fields[0], ("agent_id", "agent-1".to_string()));
    assert_eq!(fields[1], ("agent_name", "claude".to_string()));
    assert_eq!(fields[2], ("owner", "job:job-1".to_string()));
    assert_eq!(fields[3], ("workspace_path", "/work".to_string()));
    assert_eq!(fields[4], ("command", "claude".to_string()));
    assert_eq!(fields[5], ("cwd", "/work".to_string()));

    // Test SendToAgent fields
    let effect =
        Effect::SendToAgent { agent_id: AgentId::new("agent-1"), input: "hello".to_string() };
    let fields = effect.fields();
    assert_eq!(fields, vec![("agent_id", "agent-1".to_string())]);

    // Test KillAgent fields
    let effect = Effect::KillAgent { agent_id: AgentId::new("agent-1") };
    let fields = effect.fields();
    assert_eq!(fields, vec![("agent_id", "agent-1".to_string())]);

    // Test CreateWorkspace fields
    let effect = Effect::CreateWorkspace {
        workspace_id: crate::WorkspaceId::new("ws-1"),
        path: PathBuf::from("/work"),
        owner: OwnerId::Job(crate::JobId::new("job-1")),
        workspace_type: Some("folder".to_string()),
        repo_root: None,
        branch: None,
        start_point: None,
    };
    let fields = effect.fields();
    assert_eq!(fields, vec![("workspace_id", "ws-1".to_string()), ("path", "/work".to_string()),]);

    // Test DeleteWorkspace fields
    let effect = Effect::DeleteWorkspace { workspace_id: crate::WorkspaceId::new("ws-1") };
    let fields = effect.fields();
    assert_eq!(fields, vec![("workspace_id", "ws-1".to_string())]);

    // Test SetTimer fields
    let effect =
        Effect::SetTimer { id: TimerId::new("timer-1"), duration: Duration::from_millis(5000) };
    let fields = effect.fields();
    assert_eq!(
        fields,
        vec![("timer_id", "timer-1".to_string()), ("duration_ms", "5000".to_string())]
    );

    // Test CancelTimer fields
    let effect = Effect::CancelTimer { id: TimerId::new("timer-1") };
    let fields = effect.fields();
    assert_eq!(fields, vec![("timer_id", "timer-1".to_string())]);

    // Test Shell fields
    let effect = Effect::Shell {
        owner: Some(JobId::new("job-1").into()),
        step: "build".to_string(),
        command: "make".to_string(),
        cwd: PathBuf::from("/src"),
        env: HashMap::new(),
        container: None,
    };
    let fields = effect.fields();
    assert_eq!(
        fields,
        vec![
            ("owner", "job:job-1".to_string()),
            ("step", "build".to_string()),
            ("cwd", "/src".to_string())
        ]
    );

    // Test PollQueue fields
    let effect = Effect::PollQueue {
        worker_name: "fixer".to_string(),
        project: String::new(),
        list_command: "echo '[]'".to_string(),
        cwd: PathBuf::from("/work"),
    };
    let fields = effect.fields();
    assert_eq!(fields, vec![("worker", "fixer".to_string()), ("cwd", "/work".to_string())]);

    // Test TakeQueueItem fields
    let effect = Effect::TakeQueueItem {
        worker_name: "fixer".to_string(),
        project: String::new(),
        take_command: "echo taken".to_string(),
        cwd: PathBuf::from("/work"),
        item_id: "item-1".to_string(),
        item: serde_json::json!({"id": "item-1"}),
    };
    let fields = effect.fields();
    assert_eq!(
        fields,
        vec![
            ("worker", "fixer".to_string()),
            ("cwd", "/work".to_string()),
            ("item_id", "item-1".to_string()),
        ]
    );

    // Test Notify fields
    let effect = Effect::Notify { title: "Build".to_string(), message: "Done".to_string() };
    let fields = effect.fields();
    assert_eq!(fields, vec![("title", "Build".to_string())]);
}
