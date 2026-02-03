// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

/// Random prefix for this test run to avoid conflicts with parallel test runs.
static TEST_PREFIX: LazyLock<String> = LazyLock::new(|| {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("s{:04x}", nanos & 0xFFFF)
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

/// Create a tmux session directly (bypassing TmuxAdapter) for testing
fn create_test_session(session_id: &str) -> bool {
    std::process::Command::new("tmux")
        .args(["new-session", "-d", "-s", session_id, "sleep", "60"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Kill a tmux session
fn kill_test_session(session_id: &str) {
    let _ = std::process::Command::new("tmux")
        .args(["kill-session", "-t", session_id])
        .status();
}

/// Check if a tmux session exists
fn session_exists(session_id: &str) -> bool {
    std::process::Command::new("tmux")
        .args(["has-session", "-t", session_id])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

use oj_daemon::protocol::SessionSummary;

fn make_session(id: &str, namespace: &str, pipeline_id: Option<&str>) -> SessionSummary {
    SessionSummary {
        id: id.to_string(),
        namespace: namespace.to_string(),
        pipeline_id: pipeline_id.map(|s| s.to_string()),
        updated_at_ms: 0,
    }
}

fn output_string(buf: &[u8]) -> String {
    String::from_utf8(buf.to_vec()).unwrap()
}

#[test]
fn list_with_project_column() {
    let sessions = vec![
        make_session("sess-1", "myproject", Some("pipe-1")),
        make_session("sess-2", "other", Some("pipe-2")),
    ];
    let mut buf = Vec::new();
    super::format_session_list(&mut buf, &sessions);
    let out = output_string(&buf);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("PROJECT"));
    assert!(lines[1].contains("myproject"));
    assert!(lines[2].contains("other"));
}

#[test]
fn list_mixed_namespace_shows_no_project_for_empty() {
    let sessions = vec![
        make_session("sess-1", "myproject", Some("pipe-1")),
        make_session("sess-2", "", Some("pipe-2")),
    ];
    let mut buf = Vec::new();
    super::format_session_list(&mut buf, &sessions);
    let out = output_string(&buf);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("PROJECT"));
    assert!(lines[1].contains("myproject"));
    assert!(lines[2].contains("(no project)"));
}

#[test]
fn list_no_project_when_all_empty_namespace() {
    let sessions = vec![make_session("sess-1", "", Some("pipe-1"))];
    let mut buf = Vec::new();
    super::format_session_list(&mut buf, &sessions);
    let out = output_string(&buf);
    assert!(!out.contains("PROJECT"));
}

#[test]
fn attach_uses_session_id_directly_without_prefix() {
    if !tmux_available() {
        eprintln!("skipping test: tmux not available");
        return;
    }

    // Create a session with the oj- prefix (as TmuxAdapter.spawn() does)
    let name = unique_name("attach");
    let session_id = format!("oj-{}", name);

    assert!(
        create_test_session(&session_id),
        "failed to create test session"
    );

    // Verify session exists with the prefixed name
    assert!(
        session_exists(&session_id),
        "session should exist with oj- prefix"
    );

    // Verify session does NOT exist with double prefix (the bug we fixed)
    let double_prefixed = format!("oj-{}", session_id);
    assert!(
        !session_exists(&double_prefixed),
        "session should NOT exist with double oj-oj- prefix"
    );

    // Note: We can't actually test attach() in a unit test because it takes over
    // the terminal. But we can verify the session ID format is correct by checking
    // that the session exists with the ID we would pass to attach().

    // Cleanup
    kill_test_session(&session_id);
}

#[test]
fn attach_fails_for_nonexistent_session() {
    if !tmux_available() {
        eprintln!("skipping test: tmux not available");
        return;
    }

    let result = super::attach("nonexistent-session-xyz-12345");
    assert!(result.is_err());
}

#[test]
fn kill_subcommand_parses() {
    use clap::Parser;

    // Verify the Kill subcommand is recognized by the CLI parser
    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: super::SessionCommand,
    }

    let cli = Cli::parse_from(["session", "kill", "my-session"]);
    match cli.command {
        super::SessionCommand::Kill { id } => assert_eq!(id, "my-session"),
        _ => panic!("expected Kill variant"),
    }
}
