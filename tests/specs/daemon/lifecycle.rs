//! Daemon lifecycle specs
//!
//! Verify daemon start/stop/status lifecycle and crash recovery.

use crate::prelude::*;

// =============================================================================
// Recovery Tests
// =============================================================================

/// Scenario for a slow agent that sleeps for a while.
/// The sleep gives us time to kill the daemon mid-pipeline.
const SLOW_AGENT_SCENARIO: &str = r#"
name = "slow-agent"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "Running a slow task..."

[[responses.response.tool_calls]]
tool = "Bash"
input = { command = "sleep 10" }

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
run = {{ pipeline = "slow" }}

[pipeline.slow]
vars  = ["name"]

[[pipeline.slow.step]]
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

/// Tests daemon recovery mid-pipeline.
///
/// This test verifies that when the daemon crashes while a pipeline is running,
/// restarting the daemon triggers the background reconcile flow which:
/// - Detects that the tmux session exists but the agent exited
/// - Triggers the on_dead action to advance the pipeline
#[test]
fn daemon_recovers_pipeline_after_crash() {
    let temp = Project::empty();
    temp.git_init();

    // Set up scenario and runbook
    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/slow.toml",
        &slow_agent_runbook(&scenario_path),
    );

    // Start daemon and run the slow pipeline
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "slow", "recovery-test"]).passes();

    // Wait for the pipeline to reach the agent step (Running status)
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let output = temp.oj().args(&["pipeline", "list"]).passes().stdout();
        output.contains("work") && output.contains("Running")
    });
    assert!(running, "pipeline should reach the agent step");

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

    // Wait for the pipeline to complete via recovery.
    // The reconcile flow should detect the dead agent and trigger on_dead = "done"
    let done = wait_for(SPEC_WAIT_MAX_MS * 10, || {
        temp.oj()
            .args(&["pipeline", "list"])
            .passes()
            .stdout()
            .contains("Completed")
    });

    if !done {
        // Debug: print daemon log to understand failure
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(
        done,
        "pipeline should complete after daemon recovery via on_dead action"
    );

    // Verify final state
    temp.oj()
        .args(&["pipeline", "list"])
        .passes()
        .stdout_has("Completed");
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
fn daemon_status_shows_pipeline_count() {
    let temp = Project::empty();
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["daemon", "status"])
        .passes()
        .stdout_has("Pipelines:");
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

    // Start daemon and run pipeline
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "slow", "kill-test"]).passes();

    // Wait for agent to be spawned (tmux session exists)
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
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
    let gone = wait_for(SPEC_WAIT_MAX_MS, || {
        !tmux_session_exists("oj-kill-test-worker-")
    });
    assert!(
        gone,
        "tmux session should be terminated after --kill\ndaemon log:\n{}",
        temp.daemon_log()
    );
}

/// Tests that sessions survive normal daemon shutdown (no --kill).
///
/// Sessions are intentionally preserved so that long-running agents continue
/// processing. On next startup, `reconcile_state` reconnects to survivors.
#[test]
fn sessions_survive_normal_shutdown() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/slow.toml",
        &slow_agent_runbook(&scenario_path),
    );

    // Start daemon and run pipeline
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "slow", "survive-test"]).passes();

    // Wait for agent to be spawned
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        tmux_session_exists("oj-survive-test-worker-")
    });
    assert!(
        running,
        "tmux session should exist for running agent\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Stop WITHOUT --kill
    temp.oj().args(&["daemon", "stop"]).passes();

    // Verify tmux session is still alive
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
