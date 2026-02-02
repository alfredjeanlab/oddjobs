// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::traced::TracedEffect;

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
            workspace_path: PathBuf::from("/work"),
            input: HashMap::new(),
            command: "claude".to_string(),
            env: vec![("KEY".to_string(), "value".to_string())],
            cwd: Some(PathBuf::from("/work")),
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
            mode: Some("ephemeral".to_string()),
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
                workspace_path: PathBuf::from("/w"),
                input: HashMap::new(),
                command: "claude".to_string(),
                env: vec![],
                cwd: None,
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
                mode: None,
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
        workspace_path: PathBuf::from("/work"),
        input: HashMap::new(),
        command: "claude".to_string(),
        env: vec![],
        cwd: Some(PathBuf::from("/work")),
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
        mode: Some("ephemeral".to_string()),
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

    // Test Notify fields
    let effect = Effect::Notify {
        title: "Build".to_string(),
        message: "Done".to_string(),
    };
    let fields = effect.fields();
    assert_eq!(fields, vec![("title", "Build".to_string())]);
}
