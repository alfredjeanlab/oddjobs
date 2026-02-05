// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Step transition tests

use super::*;
use oj_core::JobId;

#[tokio::test]
async fn shell_failure_fails_job() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Simulate shell failure
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
    assert_eq!(job.step, "failed");
}

#[tokio::test]
async fn agent_error_fails_job() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance to plan step
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

    // Simulate agent failure via fail_job (orchestrator-driven)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.fail_job(&job, "timeout").await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "failed");
}

#[tokio::test]
async fn on_fail_transition_executes() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance to merge step (which has on_fail = "cleanup")
    // init -> plan -> execute -> merge
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

    // Advance through agent steps (plan -> execute -> merge)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "merge");

    // Simulate merge failure - should transition to cleanup (custom step)
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "merge".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // With string-based steps, custom steps like "cleanup" now work correctly
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(
        job.step, "cleanup",
        "Expected cleanup step, got {}",
        job.step
    );
}

#[tokio::test]
async fn final_step_completes_job() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance through all steps to done
    // init -> plan -> execute -> merge -> done
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

    // Advance through agent steps (plan -> execute -> merge)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "merge".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}

#[tokio::test]
async fn done_step_run_command_executes() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance through all steps: init -> plan -> execute -> merge -> done
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

    // Advance through agent steps (plan -> execute -> merge)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    ctx.runtime.advance_job(&job).await.unwrap();

    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "merge".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // At this point, job should be in Done step with Running status
    // (because the "done" step's run command is executing)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Running);

    // Complete the "done" step's shell command
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "done".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Now job should be Done with Completed status
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Completed);
}

#[tokio::test]
async fn wrong_step_shell_completed_ignored() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Try to complete a step we're not in
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "merge".to_string(), // We're in init, not merge
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Should still be in init
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "init");
}

/// Runbook without explicit on_done - step should complete the job
const RUNBOOK_NO_ON_DONE: &str = r#"
[command.simple]
args = "<name>"
run = { job = "simple" }

[job.simple]
input  = ["name"]

[[job.simple.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn step_without_on_done_completes_job() {
    let ctx = setup_with_runbook(RUNBOOK_NO_ON_DONE).await;

    // Create job
    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "simple",
            "simple",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();

    // Complete init - no on_done means job should complete, not advance sequentially
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
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Completed);
}

/// Runbook with explicit next step transitions
const RUNBOOK_EXPLICIT_NEXT: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"
on_done = "custom"

[[job.build.step]]
name = "custom"
run = "echo custom"
on_done = "done"

[[job.build.step]]
name = "done"
run = "echo done"
"#;

#[tokio::test]
async fn explicit_next_step_is_followed() {
    let ctx = setup_with_runbook(RUNBOOK_EXPLICIT_NEXT).await;

    // Create job
    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();

    // Complete init - should go to custom (not second step in order)
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
    assert_eq!(job.step, "custom");

    // Complete custom - should go to done (from explicit next)
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "custom".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "done");
}

/// Runbook where done step has no run command (implicit completion)
const RUNBOOK_IMPLICIT_DONE: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"
on_done = "done"

[[job.build.step]]
name = "done"
run = "true"
"#;

#[tokio::test]
async fn implicit_done_step_completes_immediately() {
    let ctx = setup_with_runbook(RUNBOOK_IMPLICIT_DONE).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();

    // Complete init - should advance to done step
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
    assert_eq!(job.step, "done");
    assert_eq!(job.step_status, StepStatus::Running);

    // Complete done step - job should complete
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "done".to_string(),
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

#[tokio::test]
async fn step_runs_with_fallback_workspace_path() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "init");

    // Shell completion should work even if workspace_path might not be set
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
    assert_eq!(job.step, "plan");
}

#[tokio::test]
async fn advance_job_cancels_exit_deferred_timer() {
    use oj_core::TimerId;
    use std::time::Duration;

    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance to plan step (agent)
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
    assert_eq!(job.step, "plan");

    // Manually schedule an exit-deferred timer (simulates liveness detecting death)
    {
        let scheduler = ctx.runtime.executor.scheduler();
        let mut sched = scheduler.lock();
        sched.set_timer(
            TimerId::exit_deferred(&JobId::new(job_id.clone())).to_string(),
            Duration::from_secs(5),
            ctx.clock.now(),
        );
    }

    // Advance job past the agent step
    ctx.runtime.advance_job(&job).await.unwrap();

    // Verify exit-deferred timer is cancelled
    // (liveness timer may be re-created if the next step is also an agent)
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(Duration::from_secs(3600));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        !timer_ids.contains(&TimerId::exit_deferred(&JobId::new(job_id.clone())).as_str()),
        "advance_job must cancel exit-deferred timer"
    );
}

#[tokio::test]
async fn fail_job_cancels_exit_deferred_timer() {
    use oj_core::TimerId;
    use std::time::Duration;

    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance to plan step (agent)
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
    assert_eq!(job.step, "plan");

    // Manually schedule an exit-deferred timer (simulates liveness detecting death)
    {
        let scheduler = ctx.runtime.executor.scheduler();
        let mut sched = scheduler.lock();
        sched.set_timer(
            TimerId::exit_deferred(&JobId::new(job_id.clone())).to_string(),
            Duration::from_secs(5),
            ctx.clock.now(),
        );
    }

    // Fail the job from the agent step
    ctx.runtime.fail_job(&job, "test failure").await.unwrap();

    // Verify both timers are cancelled
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(Duration::from_secs(3600));
    let fired = sched.fired_timers(ctx.clock.now());
    let timer_ids: Vec<&str> = fired
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();

    assert!(
        !timer_ids.contains(&TimerId::exit_deferred(&JobId::new(job_id.clone())).as_str()),
        "fail_job must cancel exit-deferred timer"
    );
    assert!(
        !timer_ids.contains(&TimerId::liveness(&JobId::new(job_id.clone())).as_str()),
        "fail_job must cancel liveness timer"
    );
}

// --- Job-level lifecycle hook tests ---

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

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "deploy",
            "deploy",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

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

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "deploy",
            "deploy",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

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

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "deploy",
            "deploy",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

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

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "deploy",
            "deploy",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

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

// --- Locals interpolation tests ---

/// Runbook with locals that reference job vars via ${var.*}
const RUNBOOK_WITH_LOCALS: &str = r#"
[command.build]
args = "<name> <instructions>"
run = { job = "build" }

[job.build]
input = ["name", "instructions"]

[job.build.locals]
branch = "feature/${var.name}"
title = "feat(${var.name}): ${var.instructions}"

[[job.build.step]]
name = "init"
run = "echo ${local.branch} ${local.title}"
"#;

#[tokio::test]
async fn locals_interpolate_var_references() {
    let ctx = setup_with_runbook(RUNBOOK_WITH_LOCALS).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [
                ("name".to_string(), "auth".to_string()),
                ("instructions".to_string(), "add login".to_string()),
            ]
            .into_iter()
            .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let job = ctx.runtime.get_job(&job_id).unwrap();

    assert_eq!(
        job.vars.get("local.branch").map(String::as_str),
        Some("feature/auth"),
        "local.branch should interpolate ${{var.name}}"
    );
    assert_eq!(
        job.vars.get("local.title").map(String::as_str),
        Some("feat(auth): add login"),
        "local.title should interpolate ${{var.name}} and ${{var.instructions}}"
    );
}

/// Runbook with locals that reference workspace variables
const RUNBOOK_LOCALS_WITH_WORKSPACE: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]
workspace = "folder"

[job.build.locals]
branch = "feature/${var.name}-${workspace.nonce}"

[[job.build.step]]
name = "init"
run = "echo ${local.branch}"
"#;

#[tokio::test]
async fn locals_interpolate_workspace_variables() {
    let ctx = setup_with_runbook(RUNBOOK_LOCALS_WITH_WORKSPACE).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "auth".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let job = ctx.runtime.get_job(&job_id).unwrap();

    let branch = job.vars.get("local.branch").cloned().unwrap_or_default();
    assert!(
        branch.starts_with("feature/auth-"),
        "local.branch should start with 'feature/auth-', got: {branch}"
    );
    // Should NOT contain unresolved template variables
    assert!(
        !branch.contains("${"),
        "local.branch should not contain unresolved variables, got: {branch}"
    );
}

/// Locals containing shell expressions $(...) are eagerly evaluated at job
/// creation time. The output of the shell command is stored as plain data.
const RUNBOOK_LOCALS_SHELL_SUBST: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]

[job.build.locals]
repo = "$(echo /some/repo)"

[[job.build.step]]
name = "init"
run = "echo ${local.repo}"
"#;

#[tokio::test]
async fn locals_eagerly_evaluate_shell_expressions() {
    let ctx = setup_with_runbook(RUNBOOK_LOCALS_SHELL_SUBST).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let job = ctx.runtime.get_job(&job_id).unwrap();

    // After eager evaluation, $(echo /some/repo) should be resolved
    assert_eq!(
        job.vars.get("local.repo").map(String::as_str),
        Some("/some/repo"),
        "Shell command substitution should be eagerly evaluated in locals"
    );
}

// --- Workspace variable interpolation tests ---

/// Runbook with folder workspace and variables referencing workspace.nonce
/// Tests runtime interpolation of workspace variables (not just parsing).
const RUNBOOK_WORKSPACE_VAR_INTERPOLATION: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]
workspace = "folder"

[job.build.locals]
branch = "feature/${var.name}-${workspace.nonce}"

[[job.build.step]]
name = "init"
run = "echo ${local.branch}"
"#;

#[tokio::test]
async fn workspace_variables_interpolate_at_runtime() {
    let ctx = setup_with_runbook(RUNBOOK_WORKSPACE_VAR_INTERPOLATION).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "auth".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let job = ctx.runtime.get_job(&job_id).unwrap();

    // workspace.nonce should be set
    let nonce = job.vars.get("workspace.nonce").cloned().unwrap_or_default();
    assert!(!nonce.is_empty(), "workspace.nonce should be set");

    // local.branch should be interpolated with var.name and workspace.nonce
    let branch = job.vars.get("local.branch").cloned().unwrap_or_default();
    assert!(
        branch.starts_with("feature/auth-"),
        "local.branch should start with 'feature/auth-', got: {branch}"
    );
    assert!(
        !branch.contains("${"),
        "local.branch should not contain unresolved variables, got: {branch}"
    );

    // Verify the nonce portion is in the branch
    assert!(
        branch.ends_with(&nonce),
        "local.branch should end with workspace.nonce '{nonce}', got: {branch}"
    );
}

// --- on_fail cycle tests ---

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

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

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

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

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

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

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

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

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

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

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
