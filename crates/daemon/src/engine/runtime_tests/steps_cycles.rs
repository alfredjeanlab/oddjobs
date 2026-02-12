// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! On_fail cycle preservation and circuit breaker tests

use super::*;

#[tokio::test]
async fn on_fail_self_cycle_preserves_attempts() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "build",
        "",
        &[
            ("work", "false", "on_fail = { step = \"work\" }\non_done = { step = \"done\" }"),
            ("done", "echo done", ""),
        ],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    // Set some attempts to simulate agent retry tracking
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(&job_id) {
            p.increment_attempt("exit", 0);
            p.increment_attempt("exit", 0);
        }
    });

    // Verify attempts are set
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.actions.get_action_attempt("exit", 0), 2);

    // Shell fails → on_fail = { step = "work" } (self-cycle)
    ctx.runtime.handle_event(shell_fail(&job_id, "work")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work", "should cycle back to work step");
    // attempts should be preserved across the on_fail cycle
    assert_eq!(
        job.actions.get_action_attempt("exit", 0),
        2,
        "attempts must be preserved on on_fail self-cycle"
    );
}

#[tokio::test]
async fn on_fail_multi_step_cycle_preserves_attempts() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "build",
        "",
        &[
            ("work", "false", "on_fail = { step = \"recover\" }\non_done = { step = \"done\" }"),
            ("recover", "false", "on_fail = { step = \"work\" }"),
            ("done", "echo done", ""),
        ],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
    assert_eq!(ctx.runtime.get_job(&job_id).unwrap().step, "work");

    // Set attempts to simulate prior attempts
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(&job_id) {
            p.increment_attempt("exit", 0);
        }
    });

    // work fails → on_fail → recover
    ctx.runtime.handle_event(shell_fail(&job_id, "work")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "recover");
    assert_eq!(
        job.actions.get_action_attempt("exit", 0),
        1,
        "attempts preserved after work→recover on_fail transition"
    );

    // recover fails → on_fail → work (completing the cycle)
    ctx.runtime.handle_event(shell_fail(&job_id, "recover")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert_eq!(
        job.actions.get_action_attempt("exit", 0),
        1,
        "attempts preserved across full on_fail cycle"
    );
}

#[tokio::test]
async fn on_done_transition_resets_attempts() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "build",
        "",
        &[
            ("work", "false", "on_fail = { step = \"work\" }\non_done = { step = \"done\" }"),
            ("done", "echo done", ""),
        ],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    // Set attempts
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(&job_id) {
            p.increment_attempt("exit", 0);
            p.increment_attempt("exit", 0);
        }
    });

    // Shell succeeds → on_done = { step = "done" }
    ctx.runtime.handle_event(shell_ok(&job_id, "work")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    // attempts should be reset on success transition
    assert_eq!(
        job.actions.get_action_attempt("exit", 0),
        0,
        "attempts must be reset on on_done transition"
    );
}

// --- Circuit breaker tests ---

#[tokio::test]
async fn circuit_breaker_fails_job_after_max_step_visits() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "build",
        "",
        &[
            ("work", "false", "on_fail = { step = \"retry\" }\non_done = { step = \"done\" }"),
            ("retry", "false", "on_fail = { step = \"work\" }"),
            ("done", "echo done", ""),
        ],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;
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
                job.error.as_deref().unwrap_or("").contains("circuit breaker"),
                "error should mention circuit breaker, got: {:?}",
                job.error
            );
            return;
        }

        let step = job.step.clone();
        ctx.runtime.handle_event(shell_fail(&job_id, &step)).await.unwrap();
    }

    panic!("circuit breaker never fired after 50 iterations");
}

#[tokio::test]
async fn step_visits_tracked_across_transitions() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "build",
        "",
        &[
            ("work", "false", "on_fail = { step = \"retry\" }\non_done = { step = \"done\" }"),
            ("retry", "false", "on_fail = { step = \"work\" }"),
            ("done", "echo done", ""),
        ],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "build", &[]).await;

    // Initial step "work" - step_visits not yet tracked (initial step)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.get_step_visits("work"), 0);

    // work fails → retry (visit 1 for retry)
    ctx.runtime.handle_event(shell_fail(&job_id, "work")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "retry");
    assert_eq!(job.get_step_visits("retry"), 1);

    // retry fails → work (visit 1 for work via JobAdvanced)
    ctx.runtime.handle_event(shell_fail(&job_id, "retry")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert_eq!(job.get_step_visits("work"), 1);
    assert_eq!(job.get_step_visits("retry"), 1);
}
