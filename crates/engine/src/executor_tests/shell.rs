// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for shell effect execution.

use super::*;

#[tokio::test]
async fn shell_effect_runs_command() {
    let mut harness = setup().await;

    let event = harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
            step: "init".to_string(),
            command: "echo hello".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "shell effects return None (async)");

    let completed = harness.event_rx.recv().await.unwrap();
    assert!(matches!(completed, Event::ShellExited { exit_code: 0, .. }));
}

#[tokio::test]
async fn shell_failure_returns_nonzero() {
    let mut harness = setup().await;

    let event = harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
            step: "init".to_string(),
            command: "exit 1".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "shell effects return None (async)");

    let completed = harness.event_rx.recv().await.unwrap();
    assert!(matches!(completed, Event::ShellExited { exit_code: 1, .. }));
}

#[tokio::test]
async fn shell_intermediate_failure_propagates() {
    let mut harness = setup().await;

    let event = harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
            step: "init".to_string(),
            command: "false\ntrue".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "shell effects return None (async)");

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited { exit_code, .. } => {
            assert_ne!(exit_code, 0, "intermediate failure should propagate");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_pipefail_propagates() {
    let mut harness = setup().await;

    let event = harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
            step: "init".to_string(),
            command: "exit 1 | cat".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    assert!(event.is_none(), "shell effects return None (async)");

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited { exit_code, .. } => {
            assert_ne!(exit_code, 0, "pipe failure should propagate with pipefail");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_captures_stdout_and_stderr() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
            step: "init".to_string(),
            command: "echo stdout_output && echo stderr_output >&2".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited {
            exit_code,
            stdout,
            stderr,
            ..
        } => {
            assert_eq!(exit_code, 0);
            assert_eq!(stdout.unwrap().trim(), "stdout_output");
            assert_eq!(stderr.unwrap().trim(), "stderr_output");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_with_env_vars() {
    let mut harness = setup().await;

    let mut env = HashMap::new();
    env.insert("MY_TEST_VAR".to_string(), "hello_from_env".to_string());

    harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
            step: "init".to_string(),
            command: "echo $MY_TEST_VAR".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env,
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited {
            exit_code, stdout, ..
        } => {
            assert_eq!(exit_code, 0);
            assert_eq!(stdout.unwrap().trim(), "hello_from_env");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_with_none_owner() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::Shell {
            owner: None,
            step: "init".to_string(),
            command: "echo no_owner".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited {
            exit_code, stdout, ..
        } => {
            assert_eq!(exit_code, 0);
            assert_eq!(stdout.unwrap().trim(), "no_owner");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_with_agent_run_owner() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::AgentRun(AgentRunId::new("ar-1"))),
            step: "run".to_string(),
            command: "echo agent_run_shell".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited {
            exit_code, stdout, ..
        } => {
            assert_eq!(exit_code, 0);
            assert_eq!(stdout.unwrap().trim(), "agent_run_shell");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn shell_no_stdout_when_empty() {
    let mut harness = setup().await;

    harness
        .executor
        .execute(Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("test"))),
            step: "init".to_string(),
            command: "true".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        })
        .await
        .unwrap();

    let completed = harness.event_rx.recv().await.unwrap();
    match completed {
        Event::ShellExited {
            exit_code,
            stdout,
            stderr,
            ..
        } => {
            assert_eq!(exit_code, 0);
            assert!(stdout.is_none(), "empty stdout should be None");
            assert!(stderr.is_none(), "empty stderr should be None");
        }
        other => panic!("expected ShellExited, got {:?}", other),
    }
}

#[tokio::test]
async fn execute_all_shell_effects_are_async() {
    let mut harness = setup().await;

    let effects = vec![
        Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("pipe-1"))),
            step: "init".to_string(),
            command: "echo first".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        },
        Effect::Shell {
            owner: Some(OwnerId::Job(JobId::new("pipe-1"))),
            step: "build".to_string(),
            command: "echo second".to_string(),
            cwd: std::path::PathBuf::from("/tmp"),
            env: HashMap::new(),
        },
    ];

    let inline_events = harness.executor.execute_all(effects).await.unwrap();
    assert!(
        inline_events.is_empty(),
        "shell effects produce no inline events"
    );

    // Both completions arrive via channel
    let e1 = harness.event_rx.recv().await.unwrap();
    let e2 = harness.event_rx.recv().await.unwrap();
    assert!(matches!(e1, Event::ShellExited { .. }));
    assert!(matches!(e2, Event::ShellExited { .. }));
}
