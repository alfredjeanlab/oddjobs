// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[tokio::test]
async fn fake_session_spawn() {
    let adapter = FakeSessionAdapter::new();
    let id = adapter
        .spawn(
            "test",
            Path::new("/tmp"),
            "echo hello",
            &[("KEY".to_string(), "value".to_string())],
            &[],
        )
        .await
        .unwrap();

    assert!(adapter.get_session(&id).is_some());

    let calls = adapter.calls();
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0], SessionCall::Spawn { .. }));
}

#[tokio::test]
async fn fake_session_lifecycle() {
    let adapter = FakeSessionAdapter::new();
    let id = adapter
        .spawn("test", Path::new("/tmp"), "cmd", &[], &[])
        .await
        .unwrap();

    assert!(adapter.is_alive(&id).await.unwrap());

    adapter.set_exited(&id, 0);
    assert!(!adapter.is_alive(&id).await.unwrap());
}

#[tokio::test]
async fn fake_session_send_success() {
    let adapter = FakeSessionAdapter::new();
    let id = adapter
        .spawn("test", Path::new("/tmp"), "cmd", &[], &[])
        .await
        .unwrap();

    adapter.send(&id, "input text").await.unwrap();

    let calls = adapter.calls();
    assert!(
        matches!(&calls[1], SessionCall::Send { id: sid, input } if sid == &id && input == "input text")
    );
}

#[tokio::test]
async fn fake_session_send_not_found() {
    let adapter = FakeSessionAdapter::new();
    let result = adapter.send("nonexistent", "input").await;
    assert!(matches!(result, Err(SessionError::NotFound(_))));
}

#[tokio::test]
async fn fake_session_kill() {
    let adapter = FakeSessionAdapter::new();
    let id = adapter
        .spawn("test", Path::new("/tmp"), "cmd", &[], &[])
        .await
        .unwrap();

    assert!(adapter.is_alive(&id).await.unwrap());
    adapter.kill(&id).await.unwrap();
    assert!(!adapter.is_alive(&id).await.unwrap());

    let calls = adapter.calls();
    assert!(matches!(&calls[2], SessionCall::Kill { .. }));
}

#[tokio::test]
async fn fake_session_set_output_and_capture() {
    let adapter = FakeSessionAdapter::new();
    let id = adapter
        .spawn("test", Path::new("/tmp"), "cmd", &[], &[])
        .await
        .unwrap();

    adapter.set_output(&id, vec!["line1".into(), "line2".into(), "line3".into()]);

    let output = adapter.capture_output(&id, 2).await.unwrap();
    assert_eq!(output, "line2\nline3");

    let all_output = adapter.capture_output(&id, 10).await.unwrap();
    assert_eq!(all_output, "line1\nline2\nline3");
}

#[tokio::test]
async fn fake_session_capture_output_not_found() {
    let adapter = FakeSessionAdapter::new();
    let result = adapter.capture_output("nonexistent", 10).await;
    assert!(matches!(result, Err(SessionError::NotFound(_))));
}

#[tokio::test]
async fn fake_session_set_process_running() {
    let adapter = FakeSessionAdapter::new();
    let id = adapter
        .spawn("test", Path::new("/tmp"), "cmd", &[], &[])
        .await
        .unwrap();

    assert!(adapter.is_process_running(&id, "cmd").await.unwrap());

    adapter.set_process_running(&id, false);
    assert!(!adapter.is_process_running(&id, "cmd").await.unwrap());

    adapter.set_process_running(&id, true);
    assert!(adapter.is_process_running(&id, "cmd").await.unwrap());
}

#[tokio::test]
async fn fake_session_is_process_running_not_found() {
    let adapter = FakeSessionAdapter::new();
    assert!(!adapter
        .is_process_running("nonexistent", "cmd")
        .await
        .unwrap());
}

#[tokio::test]
async fn fake_session_configure_records_call() {
    let adapter = FakeSessionAdapter::new();
    let id = adapter
        .spawn("test", Path::new("/tmp"), "cmd", &[], &[])
        .await
        .unwrap();

    let config = serde_json::json!({
        "color": "cyan",
        "title": "test",
        "status": {
            "left": "project build/check",
            "right": "abc12345"
        }
    });

    adapter.configure(&id, &config).await.unwrap();

    let calls = adapter.calls();
    let configure_calls: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, SessionCall::Configure { .. }))
        .collect();
    assert_eq!(configure_calls.len(), 1);
    if let SessionCall::Configure {
        id: call_id,
        config: call_config,
    } = &configure_calls[0]
    {
        assert_eq!(call_id, &id);
        assert_eq!(call_config, &config);
    }
}

#[tokio::test]
async fn fake_session_configure_not_found() {
    let adapter = FakeSessionAdapter::new();

    let config = serde_json::json!({"color": "red"});
    let result = adapter.configure("nonexistent", &config).await;
    assert!(matches!(result, Err(SessionError::NotFound(_))));
}

#[tokio::test]
async fn fake_session_is_alive_not_found() {
    let adapter = FakeSessionAdapter::new();
    assert!(!adapter.is_alive("nonexistent").await.unwrap());
}
