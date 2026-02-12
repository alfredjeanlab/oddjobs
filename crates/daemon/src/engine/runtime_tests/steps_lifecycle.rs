// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job-level lifecycle hook tests (on_done, on_fail, precedence)

use super::*;

#[tokio::test]
async fn job_on_fail_cleanup_marks_job_failed() {
    // Simulates the chore dispatch loop bug: step "work" fails, job routes to
    // on_fail cleanup step "reopen", reopen succeeds. Before the fix, the job
    // would complete as "done" (masking the failure). After, it should be "failed".
    let ctx = setup_with_runbook(&test_runbook_steps(
        "chore",
        "on_fail = { step = \"reopen\" }",
        &[("work", "echo work", ""), ("reopen", "echo reopen", "")],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "chore", &[]).await;

    // Fail the work step → should route to reopen via on_fail
    ctx.runtime.handle_event(shell_fail(&job_id, "work")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "reopen", "Expected on_fail to route to reopen");
    assert!(job.failing, "Expected job.failing to be set");

    // Complete the reopen cleanup step → should terminate as "failed", not "done"
    ctx.runtime.handle_event(shell_ok(&job_id, "reopen")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "failed", "Expected job to terminate as failed after on_fail cleanup");
    assert_eq!(job.step_status, StepStatus::Failed);
}

#[tokio::test]
async fn step_on_fail_cleanup_marks_job_failed() {
    // Same as above but with step-level on_fail instead of job-level.
    let ctx = setup_with_runbook(&test_runbook_steps(
        "deploy",
        "",
        &[
            ("work", "echo work", "on_fail = { step = \"rollback\" }"),
            ("rollback", "echo rollback", ""),
        ],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "deploy", &[]).await;

    // Fail the work step → routes to rollback via step-level on_fail
    ctx.runtime.handle_event(shell_fail(&job_id, "work")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "rollback");
    assert!(job.failing);

    // Complete rollback → should terminate as "failed"
    ctx.runtime.handle_event(shell_ok(&job_id, "rollback")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(
        job.step, "failed",
        "Expected job to terminate as failed after step on_fail cleanup"
    );
    assert_eq!(job.step_status, StepStatus::Failed);
}

#[tokio::test]
async fn job_on_done_routes_to_teardown() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "deploy",
        "on_done = { step = \"teardown\" }",
        &[
            ("init", "echo init", "on_done = { step = \"work\" }"),
            ("work", "echo work", ""),
            ("teardown", "echo teardown", ""),
        ],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "deploy", &[]).await;

    // Complete init -> work
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    // Complete work (no step-level on_done) -> should go to teardown via job on_done
    ctx.runtime.handle_event(shell_ok(&job_id, "work")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "teardown", "Expected job on_done to route to teardown");

    // Complete teardown (also no step-level on_done, but IS the on_done target) -> should complete
    ctx.runtime.handle_event(shell_ok(&job_id, "teardown")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Completed);
}

#[tokio::test]
async fn job_on_fail_routes_to_cleanup() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "deploy",
        "on_fail = { step = \"cleanup\" }",
        &[("init", "echo init", ""), ("cleanup", "echo cleanup", "")],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "deploy", &[]).await;

    // Fail init (no step-level on_fail) -> should go to cleanup via job on_fail
    ctx.runtime.handle_event(shell_fail(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "cleanup", "Expected job on_fail to route to cleanup");
}

#[tokio::test]
async fn step_level_on_done_takes_precedence() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "deploy",
        "on_done = { step = \"teardown\" }",
        &[
            ("init", "echo init", "on_done = { step = \"custom\" }"),
            ("custom", "echo custom", ""),
            ("teardown", "echo teardown", ""),
        ],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "deploy", &[]).await;

    // Complete init - step-level on_done = "custom" should take priority over job on_done = "teardown"
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "custom", "Step-level on_done should take precedence over job-level");
}

#[tokio::test]
async fn step_level_on_fail_takes_precedence() {
    let ctx = setup_with_runbook(&test_runbook_steps(
        "deploy",
        "on_fail = { step = \"global-cleanup\" }",
        &[
            ("init", "echo init", "on_fail = { step = \"step-cleanup\" }"),
            ("step-cleanup", "echo step-cleanup", ""),
            ("global-cleanup", "echo global-cleanup", ""),
        ],
    ))
    .await;

    let job_id = create_job_for_runbook(&ctx, "deploy", &[]).await;

    // Fail init - step-level on_fail = "step-cleanup" should take priority
    ctx.runtime.handle_event(shell_fail(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(
        job.step, "step-cleanup",
        "Step-level on_fail should take precedence over job-level"
    );
}
