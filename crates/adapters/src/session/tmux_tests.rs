// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use serial_test::serial;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

/// Random prefix for this test run to avoid conflicts with parallel test runs.
static TEST_PREFIX: LazyLock<String> = LazyLock::new(|| {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("t{:04x}", nanos & 0xFFFF)
});

/// Counter for generating unique session names across parallel tests.
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique session name for testing.
fn unique_name(suffix: &str) -> String {
    let id = SESSION_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{}-{}-{}", *TEST_PREFIX, suffix, id)
}

/// Check if tmux is available on this system
fn tmux_available() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

macro_rules! fail_if_no_tmux {
    () => {
        if !tmux_available() {
            panic!("tmux is required but not available");
        }
    };
}

// All tmux tests are serialized because some tests modify PATH which affects all others.

#[tokio::test]
#[serial(tmux)]
async fn spawn_creates_session_and_returns_id() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("spawn");

    let id = adapter
        .spawn(&name, Path::new("/tmp"), "sleep 60", &[], &[])
        .await
        .unwrap();

    assert_eq!(id, format!("oj-{}", name));

    // Cleanup
    let _ = adapter.kill(&id).await;
}

#[tokio::test]
#[serial(tmux)]
async fn spawn_with_env_passes_environment() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("env");
    let env = vec![("TEST_VAR".to_string(), "test_value".to_string())];

    let id = adapter
        .spawn(&name, Path::new("/tmp"), "echo $TEST_VAR && sleep 60", &env, &[])
        .await
        .unwrap();

    // Give the command time to execute
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let output = adapter.capture_output(&id, 10).await.unwrap();
    assert!(output.contains("test_value"));

    // Cleanup
    let _ = adapter.kill(&id).await;
}

#[tokio::test]
#[serial(tmux)]
async fn spawn_replaces_existing_session() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("replace");

    // Create first session
    let id1 = adapter
        .spawn(&name, Path::new("/tmp"), "sleep 60", &[], &[])
        .await
        .unwrap();

    // Create second session with same name (should replace)
    let id2 = adapter
        .spawn(&name, Path::new("/tmp"), "sleep 60", &[], &[])
        .await
        .unwrap();

    assert_eq!(id1, id2);
    assert!(adapter.is_alive(&id2).await.unwrap());

    // Cleanup
    let _ = adapter.kill(&id2).await;
}

#[tokio::test]
#[serial(tmux)]
async fn send_sends_keys_to_session() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("send");

    let id = adapter
        .spawn(&name, Path::new("/tmp"), "cat", &[], &[])
        .await
        .unwrap();

    // Give session time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Send input (cat will echo it back)
    adapter.send(&id, "hello").await.unwrap();
    adapter.send(&id, "Enter").await.unwrap();

    // Give command time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let output = adapter.capture_output(&id, 10).await.unwrap();
    assert!(output.contains("hello"));

    // Cleanup
    let _ = adapter.kill(&id).await;
}

#[tokio::test]
#[serial(tmux)]
async fn send_to_nonexistent_session_returns_not_found() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();

    let result = adapter.send("nonexistent-session-xyz", "test").await;
    assert!(matches!(result, Err(SessionError::NotFound(_))));
}

#[tokio::test]
#[serial(tmux)]
async fn kill_terminates_session() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("kill");

    let id = adapter
        .spawn(&name, Path::new("/tmp"), "sleep 60", &[], &[])
        .await
        .unwrap();

    assert!(adapter.is_alive(&id).await.unwrap());

    adapter.kill(&id).await.unwrap();

    // Give tmux time to clean up
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    assert!(!adapter.is_alive(&id).await.unwrap());
}

#[tokio::test]
#[serial(tmux)]
async fn kill_nonexistent_session_succeeds() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();

    // Killing a non-existent session should not error
    let result = adapter.kill("nonexistent-session-xyz").await;
    assert!(result.is_ok());
}

#[tokio::test]
#[serial(tmux)]
async fn is_alive_returns_true_for_running_session() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("alive");

    let id = adapter
        .spawn(&name, Path::new("/tmp"), "sleep 60", &[], &[])
        .await
        .unwrap();

    assert!(adapter.is_alive(&id).await.unwrap());

    // Cleanup
    let _ = adapter.kill(&id).await;
}

#[tokio::test]
#[serial(tmux)]
async fn is_alive_returns_false_for_nonexistent_session() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();

    let alive = adapter.is_alive("nonexistent-session-xyz").await.unwrap();
    assert!(!alive);
}

#[tokio::test]
#[serial(tmux)]
async fn capture_output_returns_pane_content() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("capture");

    // Use a command that outputs then stays running so we can capture
    let id = adapter
        .spawn(
            &name,
            Path::new("/tmp"),
            "echo 'capture-test-output' && sleep 60",
            &[],
            &[],
        )
        .await
        .unwrap();

    // Give command time to execute
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let output = adapter.capture_output(&id, 10).await.unwrap();
    assert!(output.contains("capture-test-output"));

    // Cleanup
    let _ = adapter.kill(&id).await;
}

#[tokio::test]
#[serial(tmux)]
async fn capture_output_nonexistent_session_returns_not_found() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();

    let result = adapter.capture_output("nonexistent-session-xyz", 10).await;
    assert!(matches!(result, Err(SessionError::NotFound(_))));
}

#[tokio::test]
#[serial(tmux)]
async fn is_process_running_detects_child_process() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("proc");

    // Use background + wait to ensure sleep is a child of bash (the pane process)
    // Without this, bash would exec the command directly, making sleep the pane itself
    let id = adapter
        .spawn(&name, Path::new("/tmp"), "bash -c 'sleep 60 & wait'", &[], &[])
        .await
        .unwrap();

    // Give process time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    let running = adapter.is_process_running(&id, "sleep").await.unwrap();
    assert!(running);

    // Cleanup
    let _ = adapter.kill(&id).await;
}

#[tokio::test]
#[serial(tmux)]
async fn is_process_running_detects_direct_pane_process() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("directproc");

    // Launch sleep directly (not via bash), so sleep IS the pane process, not a child
    let id = adapter
        .spawn(&name, Path::new("/tmp"), "sleep 60", &[], &[])
        .await
        .unwrap();

    // Give process time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    let running = adapter.is_process_running(&id, "sleep").await.unwrap();
    assert!(running, "should detect process running as the pane itself");

    // Cleanup
    let _ = adapter.kill(&id).await;
}

#[tokio::test]
#[serial(tmux)]
async fn is_process_running_returns_false_for_no_match() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("noproc");

    // Use background + wait to ensure sleep is a child of bash
    let id = adapter
        .spawn(&name, Path::new("/tmp"), "bash -c 'sleep 60 & wait'", &[], &[])
        .await
        .unwrap();

    // Give process time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    let running = adapter
        .is_process_running(&id, "nonexistent-process-xyz")
        .await
        .unwrap();
    assert!(!running);

    // Cleanup
    let _ = adapter.kill(&id).await;
}

#[tokio::test]
#[serial(tmux)]
async fn is_process_running_nonexistent_session_returns_not_found() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();

    let result = adapter
        .is_process_running("nonexistent-session-xyz", "sleep")
        .await;
    assert!(matches!(result, Err(SessionError::NotFound(_))));
}

#[tokio::test]
#[serial(tmux)]
async fn spawn_rejects_nonexistent_cwd() {
    fail_if_no_tmux!();
    let adapter = TmuxAdapter::new();
    let name = unique_name("badcwd");

    let result = adapter
        .spawn(&name, Path::new("/nonexistent/path"), "sleep 1", &[], &[])
        .await;

    assert!(matches!(result, Err(SessionError::SpawnFailed(_))));
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("working directory does not exist"),
        "Expected error about working directory, got: {}",
        err
    );
}

#[test]
fn tmux_adapter_is_zero_sized() {
    let adapter = TmuxAdapter;
    assert!(std::mem::size_of_val(&adapter) == 0);
}

// Tests below modify PATH to simulate tmux being unavailable.

#[tokio::test]
#[serial(tmux)]
async fn spawn_fails_when_tmux_unavailable() {
    use std::env;

    let original_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", "/nonexistent");

    let adapter = TmuxAdapter::new();
    let result = adapter
        .spawn("test-no-tmux", Path::new("/tmp"), "sleep 1", &[], &[])
        .await;

    env::set_var("PATH", &original_path);

    assert!(matches!(result, Err(SessionError::SpawnFailed(_))));
}

#[tokio::test]
#[serial(tmux)]
async fn send_fails_when_tmux_unavailable() {
    use std::env;

    let original_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", "/nonexistent");

    let adapter = TmuxAdapter::new();
    let result = adapter.send("any-session", "test").await;

    env::set_var("PATH", &original_path);

    assert!(matches!(result, Err(SessionError::CommandFailed(_))));
}

#[tokio::test]
#[serial(tmux)]
async fn kill_succeeds_when_tmux_unavailable() {
    use std::env;

    let original_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", "/nonexistent");

    let adapter = TmuxAdapter::new();
    let result = adapter.kill("any-session").await;

    env::set_var("PATH", &original_path);

    // kill() intentionally ignores errors (session might already be gone)
    assert!(result.is_ok());
}

#[tokio::test]
#[serial(tmux)]
async fn is_alive_fails_when_tmux_unavailable() {
    use std::env;

    let original_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", "/nonexistent");

    let adapter = TmuxAdapter::new();
    let result = adapter.is_alive("any-session").await;

    env::set_var("PATH", &original_path);

    assert!(matches!(result, Err(SessionError::CommandFailed(_))));
}

#[tokio::test]
#[serial(tmux)]
async fn capture_output_fails_when_tmux_unavailable() {
    use std::env;

    let original_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", "/nonexistent");

    let adapter = TmuxAdapter::new();
    let result = adapter.capture_output("any-session", 10).await;

    env::set_var("PATH", &original_path);

    assert!(matches!(result, Err(SessionError::CommandFailed(_))));
}

#[tokio::test]
#[serial(tmux)]
async fn is_process_running_fails_when_tmux_unavailable() {
    use std::env;

    let original_path = env::var("PATH").unwrap_or_default();
    env::set_var("PATH", "/nonexistent");

    let adapter = TmuxAdapter::new();
    let result = adapter.is_process_running("any-session", "pattern").await;

    env::set_var("PATH", &original_path);

    assert!(matches!(result, Err(SessionError::CommandFailed(_))));
}
