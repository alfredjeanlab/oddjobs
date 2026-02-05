//! Job escalation chain tests using claudeless simulator.
//!
//! Tests the full job escalation flow when an agent exhausts its on_dead
//! retry attempts and escalates to waiting for human intervention.
//!
//! Note: In -p mode, there's a race between idle detection and exit detection.
//! The agent may go idle briefly before exiting, causing on_idle to fire before
//! on_dead. Both paths lead to valid escalation, so tests accept either source.

use crate::prelude::*;

// =============================================================================
// Scenarios
// =============================================================================

/// Agent that exits immediately (via -p mode) without completing the task.
/// Used to trigger on_dead handling.
const FAILING_AGENT_SCENARIO: &str = r#"
name = "failing-agent"
trusted = true

[[responses]]
pattern = { type = "any" }

[responses.response]
text = "I encountered an error and cannot continue."

[tool_execution]
mode = "live"
"#;

// =============================================================================
// Runbooks
// =============================================================================

/// Runbook with an agent that fails (exits via -p) and has limited resume attempts.
/// After exhausting attempts, the job should escalate to waiting.
///
/// Note: on_idle defaults to escalate. In -p mode, the agent may go idle briefly
/// before exiting, creating a race between on_idle and on_dead escalation.
/// Setting on_idle = "escalate" explicitly makes this behavior intentional.
fn runbook_agent_escalate_after_retries(scenario_path: &std::path::Path) -> String {
    format!(
        r#"
[command.build]
args = "<name>"
run = {{ job = "build" }}

[job.build]
vars  = ["name"]

[[job.build.step]]
name = "work"
run = {{ agent = "worker" }}

[agent.worker]
run = "claudeless --scenario {} -p"
prompt = "Complete this task."
on_idle = "escalate"
on_dead = {{ action = "resume", attempts = 2 }}
"#,
        scenario_path.display()
    )
}

// =============================================================================
// Helpers
// =============================================================================

/// Extract the first decision ID from `oj decision list` output.
fn extract_decision_id(output: &str) -> Option<String> {
    for line in output.lines() {
        let line = line.trim();
        // Skip header lines and empty lines
        if line.is_empty() || line.starts_with("ID") || line.starts_with('-') {
            continue;
        }
        // First non-header line should have the ID as the first field
        if let Some(id) = line.split_whitespace().next() {
            // Decision IDs are hex strings
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(id.to_string());
            }
        }
    }
    None
}

// =============================================================================
// Tests
// =============================================================================

/// Tests the full job escalation chain:
/// 1. Agent triggers escalation (via on_idle or on_dead)
/// 2. Job goes to waiting status with a decision
/// 3. User resolves decision via `oj decision resolve`
/// 4. Job advances (via Done/Skip option 2) and completes
///
/// Uses claudeless with -p (print mode) which exits after one response.
/// In -p mode, there's a race between idle detection and exit detection,
/// so the decision source may be "idle" or "error". Both are valid escalation
/// paths, and option 2 (Done for idle, Skip for error) completes the step.
#[test]
fn full_escalation_chain_from_on_dead_to_decision_resolution() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/fail.toml", FAILING_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/fail.toml");
    temp.file(
        ".oj/runbooks/build.toml",
        &runbook_agent_escalate_after_retries(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test"]).passes();

    // Step 1: Wait for job to escalate to waiting
    // The escalation can come from either on_idle or on_dead depending on timing
    let waiting = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("waiting")
    });

    if !waiting {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(waiting, "job should escalate to waiting");

    // Step 2: Verify a decision was created
    let decision_list = temp.oj().args(&["decision", "list"]).passes().stdout();
    let decision_id = extract_decision_id(&decision_list);
    assert!(
        decision_id.is_some(),
        "decision should be created when job escalates, got:\n{}",
        decision_list
    );
    let decision_id = decision_id.unwrap();

    // Verify decision source is either "error" (from on_dead) or "idle" (from on_idle)
    // Both are valid escalation paths in -p mode
    let valid_source = decision_list.contains("error") || decision_list.contains("idle");
    assert!(
        valid_source,
        "decision source should be 'error' or 'idle', got:\n{}",
        decision_list
    );

    // Step 3: Resolve decision with option 2 (Done for idle, Skip for error)
    // Both complete the step and advance the job
    temp.oj()
        .args(&["decision", "resolve", &decision_id, "2"])
        .passes();

    // Step 4: Verify job completed after decision resolution
    let completed = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("completed")
    });

    if !completed {
        eprintln!(
            "=== JOB LIST ===\n{}\n=== DAEMON LOG ===\n{}\n=== END LOG ===",
            temp.oj().args(&["job", "list"]).passes().stdout(),
            temp.daemon_log()
        );
    }
    assert!(completed, "job should complete after decision is resolved");

    // Verify decision is no longer pending
    let decisions_after = temp.oj().args(&["decision", "list"]).passes().stdout();
    let short_id = &decision_id[..8.min(decision_id.len())];
    assert!(
        !decisions_after.contains(short_id),
        "decision should be removed from pending list after resolution, got:\n{}",
        decisions_after
    );
}

/// Tests that resolving an escalated decision with Retry resumes the job.
///
/// After escalation, resolving with option 1 (Retry) emits JobResume,
/// causing the agent to be respawned. The job should return to running.
#[test]
fn escalation_retry_resumes_job_to_running() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/fail.toml", FAILING_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/fail.toml");
    temp.file(
        ".oj/runbooks/build.toml",
        &runbook_agent_escalate_after_retries(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "retry-test"]).passes();

    // Wait for job to escalate to waiting
    let waiting = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("waiting")
    });
    assert!(
        waiting,
        "job should escalate to waiting\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Get decision ID
    let decision_list = temp.oj().args(&["decision", "list"]).passes().stdout();
    let decision_id =
        extract_decision_id(&decision_list).expect("decision should exist after escalation");

    // Resolve with option 1 (Retry) to resume the job
    temp.oj()
        .args(&["decision", "resolve", &decision_id, "1"])
        .passes();

    // Job should go back to running briefly as the agent is respawned
    // Then it will either escalate again (if agent fails) or complete
    // We just verify it leaves "waiting" state
    let not_waiting = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        !out.contains("waiting") || out.contains("running")
    });

    assert!(
        not_waiting,
        "job should leave waiting state after Retry resolution\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}

/// Tests that resolving an escalated decision with Cancel cancels the job.
#[test]
fn escalation_cancel_cancels_job() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/fail.toml", FAILING_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/fail.toml");
    temp.file(
        ".oj/runbooks/build.toml",
        &runbook_agent_escalate_after_retries(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "cancel-test"]).passes();

    // Wait for job to escalate to waiting
    let waiting = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("waiting")
    });
    assert!(
        waiting,
        "job should escalate to waiting\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Get decision ID
    let decision_list = temp.oj().args(&["decision", "list"]).passes().stdout();
    let decision_id =
        extract_decision_id(&decision_list).expect("decision should exist after escalation");

    // Resolve with option 3 (Cancel)
    temp.oj()
        .args(&["decision", "resolve", &decision_id, "3"])
        .passes();

    // Job should be cancelled
    let cancelled = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("cancelled")
    });

    assert!(
        cancelled,
        "job should be cancelled after Cancel resolution\njob list:\n{}\ndaemon log:\n{}",
        temp.oj().args(&["job", "list"]).passes().stdout(),
        temp.daemon_log()
    );
}

/// Tests that decision resolution with a freeform message works.
#[test]
fn escalation_resolve_with_freeform_message() {
    let temp = Project::empty();
    temp.git_init();

    temp.file(".oj/scenarios/fail.toml", FAILING_AGENT_SCENARIO);
    let scenario_path = temp.path().join(".oj/scenarios/fail.toml");
    temp.file(
        ".oj/runbooks/build.toml",
        &runbook_agent_escalate_after_retries(&scenario_path),
    );

    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "freeform-test"]).passes();

    // Wait for job to escalate to waiting
    let waiting = wait_for(SPEC_WAIT_MAX_MS * 5, || {
        temp.oj()
            .args(&["job", "list"])
            .passes()
            .stdout()
            .contains("waiting")
    });
    assert!(
        waiting,
        "job should escalate to waiting\ndaemon log:\n{}",
        temp.daemon_log()
    );

    // Get decision ID
    let decision_list = temp.oj().args(&["decision", "list"]).passes().stdout();
    let decision_id =
        extract_decision_id(&decision_list).expect("decision should exist after escalation");

    // Resolve with a freeform message (no option number)
    temp.oj()
        .args(&[
            "decision",
            "resolve",
            &decision_id,
            "-m",
            "Please try a different approach",
        ])
        .passes();

    // Decision should be resolved (job will resume with the message)
    let resolved = wait_for(SPEC_WAIT_MAX_MS * 3, || {
        let decisions = temp.oj().args(&["decision", "list"]).passes().stdout();
        let short_id = &decision_id[..8.min(decision_id.len())];
        !decisions.contains(short_id)
    });

    assert!(
        resolved,
        "decision should be resolved after freeform message\ndecision list:\n{}",
        temp.oj().args(&["decision", "list"]).passes().stdout()
    );
}
