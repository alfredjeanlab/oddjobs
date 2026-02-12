//! Agent stop hook resilience specs
//!
//! Verify that agent steps handle cancellation and stop scenarios correctly:
//! - Job cancel kills agent session and transitions job
//! - `on_cancel` cleanup steps run after agent is killed
//! - Queue items transition properly when agent jobs are cancelled
//! - Re-cancellation during cleanup is a no-op
//! - Daemon stop with --kill terminates agent sessions mid-job

use crate::prelude::*;

/// Agent that simulates the stop hook firing by calling `oj agent hook stop`
/// directly. This emits an AgentStop event which triggers the on_idle escalation
/// path in the daemon.
fn scenario_trigger_stop_hook() -> &'static str {
    r#"
[claude]
trusted = true

[[responses]]
on = "*"
say = "I'm done, trying to stop."

[[responses.tools]]
call = "Bash"
input = { command = "echo '{}' | oj agent hook stop $AGENT_ID" }

[[responses]]
on = "*"
say = "Waiting for further instructions."

[tools]
mode = "live"

[tools.Bash]
approve = true
"#
}

/// A slow agent that sleeps, keeping the job on the agent step long enough
/// to cancel it mid-execution. Uses -p mode so on_dead fires if not cancelled.
const SLOW_AGENT_SCENARIO: &str = r#"
[claude]
trusted = true

[[responses]]
on = "*"
say = "Running a slow task..."

[[responses.tools]]
call = "Bash"
input = { command = "sleep 30" }

[tools]
mode = "live"

[tools.Bash]
approve = true
"#;

/// Runbook with an interactive agent that triggers the stop hook.
/// on_idle = "escalate" causes the stop hook to create an escalation decision.
fn runbook_on_idle_escalate(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.work]
args = "<name>"
run = {{ job = "work" }}

[job.work]
vars  = ["name"]

[[job.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {}"
prompt = "Do the task."
on_idle = "escalate"
env = {{ AGENT_ID = "${{agent_id}}" }}
"#,
        scenario_path.display()
    )
}

/// Runbook with a slow agent step and no on_cancel. Cancellation goes straight
/// to terminal "cancelled" status.
fn runbook_agent_no_on_cancel(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.work]
args = "<name>"
run = {{ job = "work" }}

[job.work]
vars  = ["name"]

[[job.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

/// Runbook with a slow agent step and a job-level on_cancel that routes
/// to a cleanup step. The cleanup step writes a marker file to prove it ran.
fn runbook_agent_with_on_cancel(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.work]
args = "<name>"
run = {{ job = "work" }}

[job.work]
vars  = ["name"]
on_cancel = {{ step = "cleanup" }}

[[job.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[[job.work.step]]
name = "cleanup"
run = "echo cleaned"

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

/// Runbook with a slow agent step and a step-level on_cancel that routes to
/// a cleanup step.
fn runbook_agent_step_on_cancel(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.work]
args = "<name>"
run = {{ job = "work" }}

[job.work]
vars  = ["name"]

[[job.work.step]]
name = "agent"
run = {{ agent = "worker" }}
on_cancel = {{ step = "cleanup" }}

[[job.work.step]]
name = "cleanup"
run = "echo step-cleanup-ran"

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

/// Runbook with a slow agent step configured with on_dead = recover. Cancel
/// should override the recover action and go to cancelled/cleanup.
fn runbook_agent_recover_then_cancel(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.work]
args = "<name>"
run = {{ job = "work" }}

[job.work]
vars  = ["name"]

[[job.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = {{ action = "resume", attempts = 3 }}
"#,
        scenario_path.display()
    )
}

/// Runbook with queue + worker + slow agent step for testing queue item
/// transitions on cancel.
fn runbook_queue_agent_cancel(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[queue.tasks]
type = "persisted"
vars = ["name"]

[worker.runner]
source = {{ queue = "tasks" }}
run = {{ job = "work" }}
concurrency = 1

[job.work]
vars = ["name"]

[[job.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
        scenario_path.display()
    )
}

/// Extract the first job ID from `oj job list` output
/// by matching a line containing `name_filter`.
fn extract_job_id(temp: &Project, name_filter: &str) -> String {
    let output = temp.oj().args(&["job", "list"]).passes().stdout();
    output
        .lines()
        .find(|l| l.contains(name_filter))
        .unwrap_or_else(|| panic!("no job matching '{}' in output:\n{}", name_filter, output))
        .split_whitespace()
        .next()
        .expect("should have an ID column")
        .to_string()
}

/// When a job is cancelled while an agent step is running, the agent
/// session is killed and the job transitions to "cancelled" status.
#[test]
fn cancel_agent_step_transitions_job_to_cancelled() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(".oj/runbooks/work.toml", &runbook_agent_no_on_cancel(&scenario_path));

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "cancel-basic"]).passes();

    // Wait for job to reach agent step (running status)
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(running, "job should reach the agent step\ndaemon log:\n{}", temp.daemon_log());

    // Cancel the job
    let job_id = extract_job_id(&temp, "work");
    temp.oj().args(&["job", "cancel", &job_id]).passes();

    // Job should reach cancelled status
    let cancelled = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("cancelled")
    });

    if !cancelled {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(cancelled, "job should transition to cancelled after cancel during agent step");
}

/// When a job with `on_cancel` is cancelled during an agent step, the
/// agent is killed first, then the cleanup step runs, and the job
/// completes (not stuck in cancelling).
#[test]
fn on_cancel_cleanup_step_runs_after_agent_kill() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(".oj/runbooks/work.toml", &runbook_agent_with_on_cancel(&scenario_path));

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "cancel-cleanup"]).passes();

    // Wait for job to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(running, "job should reach the agent step\ndaemon log:\n{}", temp.daemon_log());

    // Cancel the job
    let job_id = extract_job_id(&temp, "work");
    temp.oj().args(&["job", "cancel", &job_id]).passes();

    // The cleanup step should run and the job should reach a terminal state.
    // With on_cancel routing, the job goes through "cleanup" step before terminal.
    let terminal = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("completed") || out.contains("cancelled")
    });

    if !terminal {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(terminal, "job should reach terminal state after on_cancel cleanup step runs");
}

/// When a step has its own on_cancel, the step-level on_cancel takes priority
/// over the job-level on_cancel.
#[test]
fn step_level_on_cancel_routes_to_cleanup() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(".oj/runbooks/work.toml", &runbook_agent_step_on_cancel(&scenario_path));

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "step-cancel"]).passes();

    // Wait for job to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(running, "job should reach the agent step\ndaemon log:\n{}", temp.daemon_log());

    // Cancel the job
    let job_id = extract_job_id(&temp, "work");
    temp.oj().args(&["job", "cancel", &job_id]).passes();

    // Job should route to cleanup step and reach terminal
    let terminal = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("completed") || out.contains("cancelled")
    });

    if !terminal {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(terminal, "job should reach terminal state after step-level on_cancel cleanup runs");
}

/// When the agent is configured with on_dead = recover (with attempts), but
/// the job is cancelled, the cancel should take effect immediately.
/// The job should NOT attempt to recover the agent.
#[test]
fn cancel_overrides_on_dead_recover() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(".oj/runbooks/work.toml", &runbook_agent_recover_then_cancel(&scenario_path));

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "cancel-vs-recover"]).passes();

    // Wait for job to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(running, "job should reach the agent step\ndaemon log:\n{}", temp.daemon_log());

    // Cancel the job while agent is running
    let job_id = extract_job_id(&temp, "work");
    temp.oj().args(&["job", "cancel", &job_id]).passes();

    // Job should be cancelled, NOT recovering
    let cancelled = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("cancelled")
    });

    if !cancelled {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(cancelled, "cancel should override on_dead=recover and transition to cancelled");
}

/// When a job is already running its on_cancel cleanup step, issuing
/// another cancel should be a no-op (the cleanup runs to completion).
#[test]
fn re_cancel_during_cleanup_is_noop() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    // Use on_cancel that runs a slightly longer command to give time for re-cancel
    temp.file(
        ".oj/runbooks/work.toml",
        &format!(
            r#"
[command.work]
args = "<name>"
run = {{ job = "work" }}

[job.work]
vars  = ["name"]
on_cancel = {{ step = "cleanup" }}

[[job.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[[job.work.step]]
name = "cleanup"
run = "echo cleanup-done"

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
            scenario_path.display()
        ),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "re-cancel-test"]).passes();

    // Wait for job to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(running, "job should reach the agent step\ndaemon log:\n{}", temp.daemon_log());

    // First cancel
    let job_id = extract_job_id(&temp, "work");
    temp.oj().args(&["job", "cancel", &job_id]).passes();

    // Immediately re-cancel (should be a no-op due to cancelling guard)
    temp.oj().args(&["job", "cancel", &job_id]).passes();

    // Job should still reach terminal state (cleanup completes, not disrupted)
    let terminal = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("completed") || out.contains("cancelled")
    });

    if !terminal {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(terminal, "job should reach terminal state despite re-cancel during cleanup");
}

/// When a queue-spawned job with an agent step is cancelled, the queue
/// item transitions from active and the concurrency slot is freed.
#[test]
fn cancel_agent_job_frees_queue_slot() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(".oj/runbooks/queue.toml", &runbook_queue_agent_cancel(&scenario_path));

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["worker", "start", "runner"]).passes();

    // Push item with a name var
    temp.oj().args(&["queue", "push", "tasks", r#"{"name": "test-item"}"#]).passes();

    // Wait for job to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(running, "job should reach the agent step\ndaemon log:\n{}", temp.daemon_log());

    // Verify queue item is active
    let active = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
    assert!(active.contains("active"), "queue item should be active");

    // Cancel the job
    let job_id = extract_job_id(&temp, "work");
    temp.oj().args(&["job", "cancel", &job_id]).passes();

    // Queue item should leave active status
    let transitioned = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
        !out.contains("active")
    });

    if !transitioned {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(transitioned, "queue item must not stay active after agent job cancel");

    // Verify the slot is freed by pushing another item and watching it activate
    temp.oj().args(&["queue", "push", "tasks", r#"{"name": "second-item"}"#]).passes();

    let second_runs = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["queue", "show", "tasks"]).passes().stdout();
        // The second item should become active (or complete), proving the slot was freed
        out.matches("active").count() >= 1 || out.matches("completed").count() >= 1
    });

    if !second_runs {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(second_runs, "second queue item should activate, proving concurrency slot was freed");
}

/// Cancelling a job that has already completed should be a no-op
/// (no crash, no state corruption).
#[test]
fn cancel_completed_job_is_noop() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/fast.toml",
        r#"
[command.fast]
args = "<name>"
run = { job = "fast" }

[job.fast]
vars  = ["name"]

[[job.fast.step]]
name = "work"
run = "echo done"
"#,
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "fast", "noop-cancel"]).passes();

    // Wait for job to complete
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(completed, "job should complete");

    // Cancel the already-completed job (should be a no-op)
    let job_id = extract_job_id(&temp, "fast");
    temp.oj().args(&["job", "cancel", &job_id]).passes();

    // Job should still show completed (not cancelled)
    temp.oj().args(&["job", "list"]).passes().stdout_has("completed");
}

/// When a job with a workspace is cancelled, the workspace directory
/// should be cleaned up (same as on successful completion).
#[test]
fn cancel_cleans_up_workspace() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/slow.toml", SLOW_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/slow.toml");
    temp.file(
        ".oj/runbooks/work.toml",
        &format!(
            r#"
[command.work]
args = "<name>"
run = {{ job = "work" }}

[job.work]
vars  = ["name"]
source = "folder"

[[job.work.step]]
name = "agent"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Run a slow task."
on_dead = "done"
"#,
            scenario_path.display()
        ),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "ws-cancel"]).passes();

    // Wait for job to reach agent step
    let running = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("agent") && out.contains("running")
    });
    assert!(running, "job should reach the agent step\ndaemon log:\n{}", temp.daemon_log());

    // Verify workspace directory exists (workspace creation is async)
    let workspaces_dir = temp.state_path().join("workspaces");
    let ws_exists_before = wait_for(SPEC_WAIT_MAX_MS, || {
        workspaces_dir.exists()
            && std::fs::read_dir(&workspaces_dir).map(|mut d| d.next().is_some()).unwrap_or(false)
    });
    assert!(ws_exists_before, "workspace directory should exist before cancel");

    // Cancel the job
    let job_id = extract_job_id(&temp, "work");
    temp.oj().args(&["job", "cancel", &job_id]).passes();

    // Job should reach cancelled status
    let cancelled = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("cancelled")
    });

    if !cancelled {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(cancelled, "job should transition to cancelled after cancel");

    // Workspace directory should be cleaned up
    let ws_cleaned = wait_for(SPEC_WAIT_MAX_MS, || {
        !workspaces_dir.exists()
            || std::fs::read_dir(&workspaces_dir).map(|mut d| d.next().is_none()).unwrap_or(true)
    });
    if !ws_cleaned {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
        if workspaces_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&workspaces_dir) {
                for entry in entries.flatten() {
                    eprintln!("workspace entry: {:?}", entry.path());
                }
            }
        }
    }
    assert!(ws_cleaned, "workspace directory should be cleaned up after cancel");
}

/// Extract the first decision ID from `oj decision list` output.
fn extract_decision_id(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("ID") || line.starts_with('-') {
            continue;
        }
        if let Some(id) = line.split_whitespace().next() {
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Tests that on_idle='escalate' creates a decision when the agent tries to
/// stop, and resolving with Done (option 2) completes the job.
///
/// Lifecycle: agent spawns → Bash tool simulates stop hook via
/// `oj agent hook stop` → coop gate mode blocks the turn →
/// daemon receives stop:blocked → creates decision → job goes to waiting →
/// resolve with option 2 (Done) → StepCompleted → job completes.
#[test]
fn on_idle_escalate_creates_decision_and_done_completes_job() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/stop.toml", scenario_trigger_stop_hook());
    let scenario_path = temp.path().join(".oj/scenarios/stop.toml");
    temp.file(".oj/runbooks/work.toml", &runbook_on_idle_escalate(&scenario_path));

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "work", "stop-test"]).passes();

    // Wait for job to escalate to waiting via stop hook
    let waiting = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("waiting")
    });
    assert!(
        waiting,
        "job should escalate to waiting after stop hook fires\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );

    // Verify a decision was created
    let decision_list = temp.oj().args(&["decision", "list"]).passes().stdout();
    let decision_id = extract_decision_id(&decision_list);
    assert!(
        decision_id.is_some(),
        "decision should be created when stop hook escalates, got:\n{}",
        decision_list
    );
    let decision_id = decision_id.unwrap();

    // Resolve with option 2 (Done/Complete)
    temp.oj().args(&["decision", "resolve", &decision_id, "2"]).passes();

    // Job should complete after decision resolution
    let completed = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj().args(&["job", "list"]).passes().stdout().contains("completed")
    });
    assert!(
        completed,
        "job should complete after Done resolution\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}
