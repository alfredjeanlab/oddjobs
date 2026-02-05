// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn effect_serialization_roundtrip() {
    let effects = vec![
        Effect::Emit {
            event: Event::PipelineDeleted {
                id: PipelineId::new("pipe-1"),
            },
        },
        Effect::SpawnAgent {
            agent_id: AgentId::new("agent-1"),
            agent_name: "claude".to_string(),
            pipeline_id: PipelineId::new("pipe-1"),
            agent_run_id: None,
            workspace_path: PathBuf::from("/work"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![("KEY".to_string(), "value".to_string())],
            cwd: Some(PathBuf::from("/work")),
            session_config: HashMap::new(),
        },
        Effect::SendToAgent {
            agent_id: AgentId::new("agent-1"),
            input: "hello".to_string(),
        },
        Effect::KillAgent {
            agent_id: AgentId::new("agent-1"),
        },
        Effect::SendToSession {
            session_id: SessionId::new("sess-1"),
            input: "hello".to_string(),
        },
        Effect::KillSession {
            session_id: SessionId::new("sess-1"),
        },
        Effect::CreateWorkspace {
            workspace_id: crate::WorkspaceId::new("ws-1"),
            path: PathBuf::from("/work/tree"),
            owner: Some("pipe-1".to_string()),
            workspace_type: Some("folder".to_string()),
            repo_root: None,
            branch: None,
            start_point: None,
        },
        Effect::DeleteWorkspace {
            workspace_id: crate::WorkspaceId::new("ws-1"),
        },
        Effect::SetTimer {
            id: TimerId::new("timer-1"),
            duration: Duration::from_secs(60),
        },
        Effect::CancelTimer {
            id: TimerId::new("timer-1"),
        },
        Effect::Shell {
            pipeline_id: PipelineId::new("pipe-1"),
            step: "init".to_string(),
            command: "echo hello".to_string(),
            cwd: PathBuf::from("/tmp"),
            env: [("KEY".to_string(), "value".to_string())]
                .into_iter()
                .collect(),
        },
        Effect::PollQueue {
            worker_name: "fixer".to_string(),
            list_command: "echo '[]'".to_string(),
            cwd: PathBuf::from("/work"),
        },
        Effect::TakeQueueItem {
            worker_name: "fixer".to_string(),
            take_command: "echo taken".to_string(),
            cwd: PathBuf::from("/work"),
            item: serde_json::json!({"id": "item-1", "title": "test"}),
        },
        Effect::Notify {
            title: "Build complete".to_string(),
            message: "Success!".to_string(),
        },
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
        (
            Effect::Emit {
                event: Event::Shutdown,
            },
            "emit",
        ),
        (
            Effect::SpawnAgent {
                agent_id: AgentId::new("a"),
                agent_name: "claude".to_string(),
                pipeline_id: PipelineId::new("p"),
                agent_run_id: None,
                workspace_path: PathBuf::from("/w"),
                input: HashMap::new(),
                command: "claude".to_string(),
                env: vec![],
                cwd: None,
                session_config: HashMap::new(),
            },
            "spawn_agent",
        ),
        (
            Effect::SendToAgent {
                agent_id: AgentId::new("a"),
                input: "i".to_string(),
            },
            "send_to_agent",
        ),
        (
            Effect::KillAgent {
                agent_id: AgentId::new("a"),
            },
            "kill_agent",
        ),
        (
            Effect::SendToSession {
                session_id: SessionId::new("s"),
                input: "i".to_string(),
            },
            "send_to_session",
        ),
        (
            Effect::KillSession {
                session_id: SessionId::new("s"),
            },
            "kill_session",
        ),
        (
            Effect::CreateWorkspace {
                workspace_id: crate::WorkspaceId::new("ws"),
                path: PathBuf::from("/p"),
                owner: None,
                workspace_type: None,
                repo_root: None,
                branch: None,
                start_point: None,
            },
            "create_workspace",
        ),
        (
            Effect::DeleteWorkspace {
                workspace_id: crate::WorkspaceId::new("ws"),
            },
            "delete_workspace",
        ),
        (
            Effect::SetTimer {
                id: TimerId::new("t"),
                duration: Duration::from_secs(1),
            },
            "set_timer",
        ),
        (
            Effect::CancelTimer {
                id: TimerId::new("t"),
            },
            "cancel_timer",
        ),
        (
            Effect::Shell {
                pipeline_id: PipelineId::new("p"),
                step: "init".to_string(),
                command: "cmd".to_string(),
                cwd: PathBuf::from("/"),
                env: HashMap::new(),
            },
            "shell",
        ),
        (
            Effect::PollQueue {
                worker_name: "w".to_string(),
                list_command: "cmd".to_string(),
                cwd: PathBuf::from("/"),
            },
            "poll_queue",
        ),
        (
            Effect::TakeQueueItem {
                worker_name: "w".to_string(),
                take_command: "cmd".to_string(),
                cwd: PathBuf::from("/"),
                item: serde_json::json!({}),
            },
            "take_queue_item",
        ),
        (
            Effect::Notify {
                title: "t".to_string(),
                message: "m".to_string(),
            },
            "notify",
        ),
    ];

    for (effect, expected_name) in cases {
        assert_eq!(effect.name(), expected_name);
    }
}

#[test]
fn traced_effect_fields() {
    // Test Emit fields
    let effect = Effect::Emit {
        event: Event::PipelineDeleted {
            id: PipelineId::new("pipe-1"),
        },
    };
    let fields = effect.fields();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].0, "event");

    // Test SpawnAgent fields
    let effect = Effect::SpawnAgent {
        agent_id: AgentId::new("agent-1"),
        agent_name: "claude".to_string(),
        pipeline_id: PipelineId::new("pipe-1"),
        agent_run_id: None,
        workspace_path: PathBuf::from("/work"),
        input: HashMap::new(),
        command: "claude".to_string(),
        env: vec![],
        cwd: Some(PathBuf::from("/work")),
        session_config: HashMap::new(),
    };
    let fields = effect.fields();
    assert_eq!(fields.len(), 6);
    assert_eq!(fields[0], ("agent_id", "agent-1".to_string()));
    assert_eq!(fields[1], ("agent_name", "claude".to_string()));
    assert_eq!(fields[2], ("pipeline_id", "pipe-1".to_string()));
    assert_eq!(fields[3], ("workspace_path", "/work".to_string()));
    assert_eq!(fields[4], ("command", "claude".to_string()));
    assert_eq!(fields[5], ("cwd", "/work".to_string()));

    // Test SendToAgent fields
    let effect = Effect::SendToAgent {
        agent_id: AgentId::new("agent-1"),
        input: "hello".to_string(),
    };
    let fields = effect.fields();
    assert_eq!(fields, vec![("agent_id", "agent-1".to_string())]);

    // Test KillAgent fields
    let effect = Effect::KillAgent {
        agent_id: AgentId::new("agent-1"),
    };
    let fields = effect.fields();
    assert_eq!(fields, vec![("agent_id", "agent-1".to_string())]);

    // Test SendToSession fields
    let effect = Effect::SendToSession {
        session_id: SessionId::new("sess-1"),
        input: "hello".to_string(),
    };
    let fields = effect.fields();
    assert_eq!(fields, vec![("session_id", "sess-1".to_string())]);

    // Test KillSession fields
    let effect = Effect::KillSession {
        session_id: SessionId::new("sess-1"),
    };
    let fields = effect.fields();
    assert_eq!(fields, vec![("session_id", "sess-1".to_string())]);

    // Test CreateWorkspace fields
    let effect = Effect::CreateWorkspace {
        workspace_id: crate::WorkspaceId::new("ws-1"),
        path: PathBuf::from("/work"),
        owner: Some("pipe-1".to_string()),
        workspace_type: Some("folder".to_string()),
        repo_root: None,
        branch: None,
        start_point: None,
    };
    let fields = effect.fields();
    assert_eq!(
        fields,
        vec![
            ("workspace_id", "ws-1".to_string()),
            ("path", "/work".to_string()),
        ]
    );

    // Test DeleteWorkspace fields
    let effect = Effect::DeleteWorkspace {
        workspace_id: crate::WorkspaceId::new("ws-1"),
    };
    let fields = effect.fields();
    assert_eq!(fields, vec![("workspace_id", "ws-1".to_string())]);

    // Test SetTimer fields
    let effect = Effect::SetTimer {
        id: TimerId::new("timer-1"),
        duration: Duration::from_millis(5000),
    };
    let fields = effect.fields();
    assert_eq!(
        fields,
        vec![
            ("timer_id", "timer-1".to_string()),
            ("duration_ms", "5000".to_string())
        ]
    );

    // Test CancelTimer fields
    let effect = Effect::CancelTimer {
        id: TimerId::new("timer-1"),
    };
    let fields = effect.fields();
    assert_eq!(fields, vec![("timer_id", "timer-1".to_string())]);

    // Test Shell fields
    let effect = Effect::Shell {
        pipeline_id: PipelineId::new("pipe-1"),
        step: "build".to_string(),
        command: "make".to_string(),
        cwd: PathBuf::from("/src"),
        env: HashMap::new(),
    };
    let fields = effect.fields();
    assert_eq!(
        fields,
        vec![
            ("pipeline_id", "pipe-1".to_string()),
            ("step", "build".to_string()),
            ("cwd", "/src".to_string())
        ]
    );

    // Test PollQueue fields
    let effect = Effect::PollQueue {
        worker_name: "fixer".to_string(),
        list_command: "echo '[]'".to_string(),
        cwd: PathBuf::from("/work"),
    };
    let fields = effect.fields();
    assert_eq!(
        fields,
        vec![
            ("worker_name", "fixer".to_string()),
            ("cwd", "/work".to_string())
        ]
    );

    // Test TakeQueueItem fields
    let effect = Effect::TakeQueueItem {
        worker_name: "fixer".to_string(),
        take_command: "echo taken".to_string(),
        cwd: PathBuf::from("/work"),
        item: serde_json::json!({"id": "item-1"}),
    };
    let fields = effect.fields();
    assert_eq!(
        fields,
        vec![
            ("worker_name", "fixer".to_string()),
            ("cwd", "/work".to_string())
        ]
    );

    // Test Notify fields
    let effect = Effect::Notify {
        title: "Build".to_string(),
        message: "Done".to_string(),
    };
    let fields = effect.fields();
    assert_eq!(fields, vec![("title", "Build".to_string())]);
}

#[test]
fn spawn_agent_session_config_roundtrip() {
    let mut session_config = HashMap::new();
    session_config.insert(
        "tmux".to_string(),
        serde_json::json!({
            "color": "cyan",
            "title": "test",
            "status": {
                "left": "project build/check",
                "right": "abc12345"
            }
        }),
    );

    let effect = Effect::SpawnAgent {
        agent_id: AgentId::new("agent-1"),
        agent_name: "claude".to_string(),
        pipeline_id: PipelineId::new("pipe-1"),
        agent_run_id: None,
        workspace_path: PathBuf::from("/work"),
        input: HashMap::new(),
        command: "claude".to_string(),
        env: vec![],
        cwd: None,
        session_config: session_config.clone(),
    };

    let json = serde_json::to_string(&effect).unwrap();
    let parsed: Effect = serde_json::from_str(&json).unwrap();

    if let Effect::SpawnAgent {
        session_config: parsed_config,
        ..
    } = parsed
    {
        assert_eq!(parsed_config, session_config);
        let tmux = parsed_config.get("tmux").unwrap();
        assert_eq!(tmux["color"], "cyan");
        assert_eq!(tmux["status"]["left"], "project build/check");
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn spawn_agent_empty_session_config_skipped_in_serialization() {
    let effect = Effect::SpawnAgent {
        agent_id: AgentId::new("agent-1"),
        agent_name: "claude".to_string(),
        pipeline_id: PipelineId::new("pipe-1"),
        agent_run_id: None,
        workspace_path: PathBuf::from("/work"),
        input: HashMap::new(),
        command: "claude".to_string(),
        env: vec![],
        cwd: None,
        session_config: HashMap::new(),
    };

    let json = serde_json::to_string(&effect).unwrap();
    assert!(
        !json.contains("session_config"),
        "empty session_config should be skipped in serialization, got: {}",
        json
    );

    // Should still round-trip correctly
    let parsed: Effect = serde_json::from_str(&json).unwrap();
    if let Effect::SpawnAgent { session_config, .. } = parsed {
        assert!(session_config.is_empty());
    } else {
        panic!("Expected SpawnAgent effect");
    }
}
