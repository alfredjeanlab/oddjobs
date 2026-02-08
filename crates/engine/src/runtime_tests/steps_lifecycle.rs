// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Job-level lifecycle hook tests (on_done, on_fail, precedence)

use super::*;
use oj_core::JobId;

/// Runbook with job-level on_done
const RUNBOOK_JOB_ON_DONE: &str = r#"
[command.deploy]
args = "<name>"
run = { job = "deploy" }

[job.deploy]
input  = ["name"]
on_done = "teardown"

[[job.deploy.step]]
name = "init"
run = "echo init"
on_done = "work"

[[job.deploy.step]]
name = "work"
run = "echo work"

[[job.deploy.step]]
name = "teardown"
run = "echo teardown"
"#;

#[tokio::test]
async fn job_on_done_routes_to_teardown() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ON_DONE).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "deploy",
            "deploy",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();

    // Complete init -> work
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    // Complete work (no step-level on_done) -> should go to teardown via job on_done
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
    assert_eq!(
        job.step, "teardown",
        "Expected job on_done to route to teardown"
    );

    // Complete teardown (also no step-level on_done, but IS the on_done target) -> should complete
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "teardown".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Completed);
}

/// Runbook with job-level on_fail
const RUNBOOK_JOB_ON_FAIL: &str = r#"
[command.deploy]
args = "<name>"
run = { job = "deploy" }

[job.deploy]
input  = ["name"]
on_fail = "cleanup"

[[job.deploy.step]]
name = "init"
run = "echo init"

[[job.deploy.step]]
name = "cleanup"
run = "echo cleanup"
"#;

#[tokio::test]
async fn job_on_fail_routes_to_cleanup() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_ON_FAIL).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "deploy",
            "deploy",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();

    // Fail init (no step-level on_fail) -> should go to cleanup via job on_fail
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "init".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(
        job.step, "cleanup",
        "Expected job on_fail to route to cleanup"
    );
}

/// Runbook where step-level on_done overrides job-level on_done
const RUNBOOK_STEP_ON_DONE_PRECEDENCE: &str = r#"
[command.deploy]
args = "<name>"
run = { job = "deploy" }

[job.deploy]
input  = ["name"]
on_done = "teardown"

[[job.deploy.step]]
name = "init"
run = "echo init"
on_done = "custom"

[[job.deploy.step]]
name = "custom"
run = "echo custom"

[[job.deploy.step]]
name = "teardown"
run = "echo teardown"
"#;

#[tokio::test]
async fn step_level_on_done_takes_precedence() {
    let ctx = setup_with_runbook(RUNBOOK_STEP_ON_DONE_PRECEDENCE).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "deploy",
            "deploy",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();

    // Complete init - step-level on_done = "custom" should take priority over job on_done = "teardown"
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(
        job.step, "custom",
        "Step-level on_done should take precedence over job-level"
    );
}

/// Runbook where step-level on_fail overrides job-level on_fail
const RUNBOOK_STEP_ON_FAIL_PRECEDENCE: &str = r#"
[command.deploy]
args = "<name>"
run = { job = "deploy" }

[job.deploy]
input  = ["name"]
on_fail = "global-cleanup"

[[job.deploy.step]]
name = "init"
run = "echo init"
on_fail = "step-cleanup"

[[job.deploy.step]]
name = "step-cleanup"
run = "echo step-cleanup"

[[job.deploy.step]]
name = "global-cleanup"
run = "echo global-cleanup"
"#;

#[tokio::test]
async fn step_level_on_fail_takes_precedence() {
    let ctx = setup_with_runbook(RUNBOOK_STEP_ON_FAIL_PRECEDENCE).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "deploy",
            "deploy",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();

    // Fail init - step-level on_fail = "step-cleanup" should take priority
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "init".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(
        job.step, "step-cleanup",
        "Step-level on_fail should take precedence over job-level"
    );
}
