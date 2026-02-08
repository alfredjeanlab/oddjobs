// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! On_fail cycle preservation and circuit breaker tests

use super::*;
use oj_core::JobId;

/// Runbook with on_fail self-cycle: step retries itself on failure
const RUNBOOK_ON_FAIL_SELF_CYCLE: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]

[[job.build.step]]
name = "work"
run = "false"
on_fail = "work"
on_done = "done"

[[job.build.step]]
name = "done"
run = "echo done"
"#;

#[tokio::test]
async fn on_fail_self_cycle_preserves_action_attempts() {
    let ctx = setup_with_runbook(RUNBOOK_ON_FAIL_SELF_CYCLE).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();

    // Set some action_attempts to simulate agent retry tracking
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(&job_id) {
            p.increment_action_attempt("exit", 0);
            p.increment_action_attempt("exit", 0);
        }
    });

    // Verify attempts are set
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.get_action_attempt("exit", 0), 2);

    // Shell fails → on_fail = "work" (self-cycle)
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "work".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work", "should cycle back to work step");
    // action_attempts should be preserved across the on_fail cycle
    assert_eq!(
        job.get_action_attempt("exit", 0),
        2,
        "action_attempts must be preserved on on_fail self-cycle"
    );
}

/// Runbook with multi-step on_fail cycle: A fails→B, B fails→A
const RUNBOOK_ON_FAIL_MULTI_STEP_CYCLE: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]

[[job.build.step]]
name = "work"
run = "false"
on_fail = "recover"
on_done = "done"

[[job.build.step]]
name = "recover"
run = "false"
on_fail = "work"

[[job.build.step]]
name = "done"
run = "echo done"
"#;

#[tokio::test]
async fn on_fail_multi_step_cycle_preserves_action_attempts() {
    let ctx = setup_with_runbook(RUNBOOK_ON_FAIL_MULTI_STEP_CYCLE).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "work");

    // Set action_attempts to simulate prior attempts
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(&job_id) {
            p.increment_action_attempt("exit", 0);
        }
    });

    // work fails → on_fail → recover
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "work".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "recover");
    assert_eq!(
        job.get_action_attempt("exit", 0),
        1,
        "action_attempts preserved after work→recover on_fail transition"
    );

    // recover fails → on_fail → work (completing the cycle)
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "recover".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert_eq!(
        job.get_action_attempt("exit", 0),
        1,
        "action_attempts preserved across full on_fail cycle"
    );
}

#[tokio::test]
async fn on_done_transition_resets_action_attempts() {
    let ctx = setup_with_runbook(RUNBOOK_ON_FAIL_SELF_CYCLE).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();

    // Set action_attempts
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(&job_id) {
            p.increment_action_attempt("exit", 0);
            p.increment_action_attempt("exit", 0);
        }
    });

    // Shell succeeds → on_done = "done"
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "work".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    // action_attempts should be reset on success transition
    assert_eq!(
        job.get_action_attempt("exit", 0),
        0,
        "action_attempts must be reset on on_done transition"
    );
}

// --- Circuit breaker tests ---

/// Runbook with a cycle: work fails→retry, retry fails→work
const RUNBOOK_CYCLE_CIRCUIT_BREAKER: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]

[[job.build.step]]
name = "work"
run = "false"
on_fail = "retry"
on_done = "done"

[[job.build.step]]
name = "retry"
run = "false"
on_fail = "work"

[[job.build.step]]
name = "done"
run = "echo done"
"#;

#[tokio::test]
async fn circuit_breaker_fails_job_after_max_step_visits() {
    let ctx = setup_with_runbook(RUNBOOK_CYCLE_CIRCUIT_BREAKER).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "work");

    // Drive the cycle: work→retry→work→retry→... until circuit breaker fires.
    // Each full cycle visits both "work" and "retry" once.
    // MAX_STEP_VISITS = 5, so after 5 visits to "work" the 6th should be blocked.
    // Initial visit to "work" doesn't count (it's the initial step, before JobAdvanced).
    // Cycle: work(fail) → retry(visit 1) → retry(fail) → work(visit 1) → ...
    let max = oj_core::job::MAX_STEP_VISITS;
    for i in 0..50 {
        let job = ctx.runtime.get_job(&job_id).unwrap();
        if job.is_terminal() {
            // Circuit breaker should fire well before 50 iterations
            assert!(
                i <= (max as usize + 1) * 2,
                "circuit breaker should have fired by now (iteration {i})"
            );
            assert_eq!(job.step, "failed");
            assert!(
                job.error
                    .as_deref()
                    .unwrap_or("")
                    .contains("circuit breaker"),
                "error should mention circuit breaker, got: {:?}",
                job.error
            );
            return;
        }

        let step = job.step.clone();
        ctx.runtime
            .handle_event(Event::ShellExited {
                job_id: JobId::new(job_id.clone()),
                step,
                exit_code: 1,
                stdout: None,
                stderr: None,
            })
            .await
            .unwrap();
    }

    panic!("circuit breaker never fired after 50 iterations");
}

#[tokio::test]
async fn step_visits_tracked_across_transitions() {
    let ctx = setup_with_runbook(RUNBOOK_CYCLE_CIRCUIT_BREAKER).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();

    // Initial step "work" - step_visits not yet tracked (initial step)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.get_step_visits("work"), 0);

    // work fails → retry (visit 1 for retry)
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "work".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "retry");
    assert_eq!(job.get_step_visits("retry"), 1);

    // retry fails → work (visit 1 for work via JobAdvanced)
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "retry".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert_eq!(job.get_step_visits("work"), 1);
    assert_eq!(job.get_step_visits("retry"), 1);
}
