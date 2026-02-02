// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tmux process utilities.

use std::sync::Arc;

use parking_lot::Mutex;

use oj_storage::MaterializedState;

/// Capture tmux pane output for a session.
///
/// When `with_color` is true, includes ANSI escape sequences (via tmux -e flag).
pub(super) async fn capture_tmux_pane(
    session_id: &str,
    with_color: bool,
) -> Result<String, String> {
    let mut args = vec!["capture-pane", "-t", session_id, "-p", "-S", "-40"];
    if with_color {
        args.push("-e");
    }

    let output = tokio::process::Command::new("tmux")
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("failed to run tmux: {}", e))?;

    if !output.status.success() {
        return Err(format!("Session not found: {}", session_id));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Kill sessions tracked by this daemon instance, concurrently.
///
/// Uses `state.sessions` (not `tmux list-sessions`) to scope kills to exactly
/// the sessions created by this daemon — safe for parallel test runs where
/// multiple daemons may be active. Each kill is spawned as a tokio task for
/// O(1) latency regardless of session count.
///
/// Uses unbuffered stderr writes instead of tracing because the non-blocking
/// tracing appender may not flush before the CLI's exit timer force-kills
/// the daemon process.
pub(super) async fn kill_state_sessions(state: &Arc<Mutex<MaterializedState>>) {
    use std::io::Write;

    let session_ids: Vec<String> = {
        let state = state.lock();
        state.sessions.keys().cloned().collect()
    };

    if session_ids.is_empty() {
        return;
    }

    let count = session_ids.len();
    let mut handles = Vec::with_capacity(count);
    for id in &session_ids {
        let id = id.clone();
        handles.push(tokio::spawn(async move {
            let _ = tokio::process::Command::new("tmux")
                .args(["kill-session", "-t", &id])
                .output()
                .await;
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }

    // Unbuffered write — survives force-kill better than tracing
    let _ = writeln!(
        std::io::stderr(),
        "ojd: killed {} sessions on shutdown",
        count
    );
}
