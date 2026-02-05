//! Daemon lifecycle specs
//!
//! Verify daemon start/stop/status lifecycle and crash recovery.

use crate::prelude::*;

// =============================================================================
// Recovery Tests
// =============================================================================

/// Scenario for a slow agent that sleeps for a while.
/// The sleep gives us time to kill the daemon mid-job.
const SLOW_AGENT_SCENARIO: &str = r#"
name = "slow-agent"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Running a slow task..."

[[responses.response.tool_calls]]
tool = "Bash"
input = { command = "sleep 1" }

[tool_execution]
mode = "live"

[tool_execution.tools.Bash]
auto_approve = true
"#;

/// Runbook with a slow agent step that uses on_dead = "done".
fn slow_agent_runbook(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.slow]
args = "<name>"
run = {{ job = "slow" }}

[job.slow]
vars  = ["name"]

[[job.slow.step]]
name = "work"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

/// Tests daemon recovery mid-job.
///
/// This test verifies that when the daemon crashes while a job is running,
/// restarting the daemon triggers the background reconcile flow which:
/// - Detects that the tmux session exists but the agent exited
/// - Triggers the on_dead action to advance the job
#[test]
fn daemon_recovers_job_after_crash() {
    let temp = Project::empty();
    temp.git_init();

    // Set up scenario and runbook
    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/slow.toml",
        &slow_agent_runbook(&scenario_path),
    );

    // Start daemon and run the slow job
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "slow", "recovery-test"]).passes();

    // Wait for the job to reach the agent step (Running status)
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp.oj().args(&["job", "list"]).passes().stdout();
        output.contains("work") && output.contains("running")
    });
    assert!(running, "job should reach the agent step");

    // Kill the daemon with SIGKILL (simulates crash - no graceful shutdown)
    let killed = temp.daemon_kill();
    assert!(killed, "should be able to kill daemon");

    // Wait for daemon to actually die
    let daemon_dead = wait_for(SPEC_WAIT_MAX_MS, || {
        // Try to connect - should fail if daemon is dead
        !temp
            .oj()
            .args(&["daemon", "status"])
            .passes()
            .stdout()
            .contains("Status: running")
    });
    assert!(daemon_dead, "daemon should be dead after kill");

    // Restart the daemon - this triggers background reconciliation
    temp.oj().args(&["daemon", "start"]).passes();

    // Wait for the job to complete via recovery.
    // The reconcile flow should detect the dead agent and trigger on_dead = "done"
    let done = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !done {
        // Debug: print daemon log to understand failure
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        done,
        "job should complete after daemon recovery via on_dead action"
    );

    // Verify final state
    temp.oj()
        .args(&["job", "list"])
        .passes()
        .stdout_has("completed");
}

#[test]
fn daemon_status_fails_when_not_running() {
    let temp = Project::empty();

    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Daemon not running");
}

#[test]
fn daemon_start_reports_success() {
    let temp = Project::empty();

    temp.oj()
        .args(&["daemon", "start"])
        .passes()
        .stdout_has("Daemon started");
}

#[test]
fn daemon_status_shows_running_after_start() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Status: running");
}

#[test]
fn daemon_status_shows_uptime() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Uptime:");
}

#[test]
fn daemon_status_shows_job_count() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Jobs:");
}

#[test]
fn daemon_status_shows_version() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Version:");
}

#[test]
fn daemon_stop_reports_success() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["daemon", "stop"])
        .passes()
        .stdout_has("Daemon stopped");
}

#[test]
fn daemon_status_fails_after_stop() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["daemon", "stop"]).passes();
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Daemon not running");
}

#[test]
fn daemon_run_shows_runbook_error() {
    let temp = Project::empty();
    temp.git_init();
    // Invalid runbook - missing required 'run' field
    temp.file(
        ".oj/runbooks/bad.toml",
        "[command.test]\nargs = \"<name>\"\n",
    );

    // Daemon starts fine (user-level, runbook not loaded yet)
    temp.oj().args(&["daemon", "start"]).passes();

    // Run command should fail with parse error (runbook loaded on-demand)
    temp.oj()
        .args(&["run", "test", "foo"])
        .fails()
        .stderr_has("skipped due to errors")
        .stderr_has("missing field");
}

#[test]
fn daemon_creates_version_file() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();

    // Daemon state files are at {OJ_STATE_DIR}/
    let version_file = temp.state_path().join("daemon.version");

    let has_version = wait_for(SPEC_WAIT_MAX_MS, || version_file.exists());

    assert!(has_version, "daemon.version file should exist");
}

#[test]
fn daemon_creates_pid_file() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();

    // Daemon state files are at {OJ_STATE_DIR}/
    let pid_file = temp.state_path().join("daemon.pid");

    let has_pid = wait_for(SPEC_WAIT_MAX_MS, || pid_file.exists());

    assert!(has_pid, "daemon.pid file should exist");
}

#[test]
fn daemon_creates_socket_file() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();

    // Daemon socket is at {OJ_STATE_DIR}/daemon.sock
    let socket_file = temp.state_path().join("daemon.sock");

    let has_socket = wait_for(SPEC_WAIT_MAX_MS, || socket_file.exists());

    assert!(has_socket, "daemon socket file should exist");
}

#[test]
fn daemon_start_error_log_shows_in_cli() {
    // Force socket path to exceed SUN_LEN (104 bytes on macOS)
    // Socket path will be: {OJ_STATE_DIR}/daemon.sock
    // We need total path > 104 chars
    let temp = Project::empty();

    // Create a deeply nested state directory to make socket path too long
    let long_suffix =
        "this_is_a_very_long_path_segment_to_ensure_socket_path_exceeds_sun_len_limit_on_macos";
    let long_state_dir = temp.state_path().join(long_suffix);
    std::fs::create_dir_all(&long_state_dir).unwrap();

    // Start should fail with socket path error, NOT "Connection timeout"
    cli()
        .pwd(temp.path())
        .env("OJ_STATE_DIR", &long_state_dir)
        .args(&["daemon", "start"])
        .fails()
        .stderr_has("path must be shorter than SUN_LEN")
        .stderr_lacks("Connection timeout");
}

// =============================================================================
// Lock Contention Tests
// =============================================================================

/// Running ojd directly when a daemon is already running must not disrupt it.
///
/// Regression: a failed startup used to delete the socket and lock files
/// belonging to the running daemon, making it unreachable.
#[test]
fn running_ojd_while_daemon_running_does_not_kill_it() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();

    // Verify daemon is running
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Status: running");

    // Run ojd directly — should fail (lock held) but not disrupt anything
    let ojd = ojd_binary();
    let output = std::process::Command::new(&ojd)
        .env("OJ_STATE_DIR", temp.state_path())
        .output()
        .expect("ojd should run");
    assert!(
        !output.status.success(),
        "ojd should fail when daemon is already running"
    );

    // Verify human-readable error message
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already running"),
        "stderr should contain 'already running', got: {stderr}"
    );
    assert!(
        stderr.contains("pid:"),
        "stderr should contain pid, got: {stderr}"
    );
    assert!(
        stderr.contains("version:"),
        "stderr should contain version, got: {stderr}"
    );

    // The original daemon must still be reachable
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Status: running");

    // State files must still exist
    assert!(
        temp.state_path().join("daemon.sock").exists(),
        "socket file must survive failed ojd"
    );
    assert!(
        temp.state_path().join("daemon.pid").exists(),
        "pid file must survive failed ojd"
    );
}

/// Running ojd twice after the first daemon exits should work normally.
/// This verifies the lock file is properly released when a daemon exits.
#[test]
fn ojd_starts_after_previous_daemon_stopped() {
    let temp = Project::empty();

    // Start and stop
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["daemon", "stop"]).passes();

    // Should be able to start again
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Status: running");
}

// =============================================================================
// Session Kill Tests
// =============================================================================

/// Check if any tmux session with the given prefix exists.
fn tmux_session_exists(prefix: &str) -> bool {
    let output = std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .unwrap_or_else(|_| {
            std::process::Command::new("true")
                .output()
                .expect("true should succeed")
        });
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|line| line.starts_with(prefix))
}

/// Tests that `oj daemon stop --kill` terminates all tmux sessions.
///
/// Lifecycle: start daemon → spawn agent → verify tmux session exists →
/// run `oj daemon stop --kill` → verify tmux session is gone.
#[test]
fn daemon_stop_kill_terminates_sessions() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/slow.toml",
        &slow_agent_runbook(&scenario_path),
    );

    // Start daemon and run job
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "slow", "kill-test"]).passes();

    // Wait for agent to be spawned (tmux session exists).
    // Use extended timeout — tmux session creation is slow under load.
    let running = wait_for(SPEC_WAIT_MAX_MS * 2, || {
        tmux_session_exists("oj-kill-test-worker-")
    });
    assert!(
        running,
        "tmux session should exist for running agent\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Stop with --kill flag
    temp.oj().args(&["daemon", "stop", "--kill"]).passes();

    // Verify tmux session is gone
    let gone = wait_for(SPEC_WAIT_MAX_MS * 2, || {
        !tmux_session_exists("oj-kill-test-worker-")
    });
    assert!(
        gone,
        "tmux session should be terminated after --kill\ndaemon log:\n{}",
        temp.daemon_log()
    );
}

/// Scenario for an agent that stays alive long enough to survive daemon shutdown.
/// Needs a longer sleep than SLOW_AGENT_SCENARIO (which is tuned for crash recovery)
/// because this test must: spawn agent → stop daemon → confirm session alive.
const LONG_LIVED_AGENT_SCENARIO: &str = r#"
name = "long-lived-agent"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Running a long task..."

[[responses.response.tool_calls]]
tool = "Bash"
input = { command = "sleep 10" }

[tool_execution]
mode = "live"

[tool_execution.tools.Bash]
auto_approve = true
"#;

/// Tests that sessions survive normal daemon shutdown (no --kill).
///
/// Sessions are intentionally preserved so that long-running agents continue
/// processing. On next startup, `reconcile_state` reconnects to survivors.
#[test]
fn sessions_survive_normal_shutdown() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", LONG_LIVED_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/slow.toml",
        &slow_agent_runbook(&scenario_path),
    );

    // Start daemon and run job
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "slow", "survive-test"]).passes();

    // Wait for agent to be spawned.
    // Use extended timeout — tmux session creation is slow under load.
    let running = wait_for(SPEC_WAIT_MAX_MS * 2, || {
        tmux_session_exists("oj-survive-test-worker-")
    });
    assert!(
        running,
        "tmux session should exist for running agent\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Stop WITHOUT --kill
    temp.oj().args(&["daemon", "stop"]).passes();

    // Wait for daemon to fully exit, then verify tmux session survived.
    // A single synchronous check is racy under load — the daemon may still
    // be shutting down (flushing WAL, closing sockets) when we check.
    let stopped = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj()
            .args(&["daemon", "status"])
            .passes()
            .stdout()
            .contains("not running")
    });
    assert!(stopped, "daemon should stop");

    assert!(
        tmux_session_exists("oj-survive-test-worker-"),
        "tmux session should survive normal daemon shutdown"
    );

    // Clean up: kill the surviving session manually so it doesn't leak
    let output = std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.starts_with("oj-survive-test-worker-") {
            let _ = std::process::Command::new("tmux")
                .args(["kill-session", "-t", line])
                .status();
        }
    }
}

// =============================================================================
// Migration Error Tests
// =============================================================================

/// Tests that snapshot migration errors are correctly displayed in CLI output.
///
/// When a snapshot has a version newer than the daemon supports, the daemon
/// should fail to start with a clear error message that propagates to the CLI.
#[test]
fn daemon_start_shows_migration_error_for_too_new_snapshot() {
    use std::io::Write;

    let temp = Project::empty();

    // Write a snapshot with a version that's too new (v99)
    // The daemon only supports up to CURRENT_SNAPSHOT_VERSION (currently 1)
    // The snapshot must be zstd-compressed (uncompressed fallback was removed)
    let snapshot_json = r#"{
        "v": 99,
        "seq": 1,
        "state": {
            "jobs": {},
            "sessions": {},
            "workspaces": {},
            "runbooks": {},
            "workers": {},
            "queue_items": {},
            "crons": {},
            "decisions": {},
            "agent_runs": {}
        },
        "created_at": "2025-01-01T00:00:00Z"
    }"#;
    let snapshot_path = temp.state_path().join("snapshot.json");
    std::fs::create_dir_all(temp.state_path()).unwrap();

    // Write zstd-compressed snapshot
    let file = std::fs::File::create(&snapshot_path).unwrap();
    let mut encoder = zstd::stream::Encoder::new(file, 3).unwrap();
    encoder.write_all(snapshot_json.as_bytes()).unwrap();
    encoder.finish().unwrap();

    // Daemon start should fail with a migration error
    temp.oj()
        .args(&["daemon", "start"])
        .fails()
        .stderr_has("snapshot version 99 is newer than supported")
        .stderr_lacks("Connection timeout");
}
