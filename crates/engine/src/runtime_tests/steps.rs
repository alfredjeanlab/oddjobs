// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Step transition tests

use super::*;
use oj_core::PipelineId;

#[tokio::test]
async fn shell_failure_fails_pipeline() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Simulate shell failure
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "failed");
}

#[tokio::test]
async fn agent_error_fails_pipeline() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to plan step
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    // Simulate agent failure via fail_pipeline (orchestrator-driven)
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    ctx.runtime
        .fail_pipeline(&pipeline, "timeout")
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "failed");
}

#[tokio::test]
async fn on_fail_transition_executes() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to merge step (which has on_fail = "cleanup")
    // init -> plan -> execute -> merge
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    // Advance through agent steps (plan -> execute -> merge)
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    ctx.runtime.advance_pipeline(&pipeline).await.unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    ctx.runtime.advance_pipeline(&pipeline).await.unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "merge");

    // Simulate merge failure - should transition to cleanup (custom step)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "merge".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    // With string-based steps, custom steps like "cleanup" now work correctly
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(
        pipeline.step, "cleanup",
        "Expected cleanup step, got {}",
        pipeline.step
    );
}

#[tokio::test]
async fn final_step_completes_pipeline() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance through all steps to done
    // init -> plan -> execute -> merge -> done
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    // Advance through agent steps (plan -> execute -> merge)
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    ctx.runtime.advance_pipeline(&pipeline).await.unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    ctx.runtime.advance_pipeline(&pipeline).await.unwrap();

    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "merge".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
}

#[tokio::test]
async fn done_step_run_command_executes() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance through all steps: init -> plan -> execute -> merge -> done
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    // Advance through agent steps (plan -> execute -> merge)
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    ctx.runtime.advance_pipeline(&pipeline).await.unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    ctx.runtime.advance_pipeline(&pipeline).await.unwrap();

    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "merge".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    // At this point, pipeline should be in Done step with Running status
    // (because the "done" step's run command is executing)
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
    assert_eq!(pipeline.step_status, StepStatus::Running);

    // Complete the "done" step's shell command
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "done".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    // Now pipeline should be Done with Completed status
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
    assert_eq!(pipeline.step_status, StepStatus::Completed);
}

#[tokio::test]
async fn wrong_step_shell_completed_ignored() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Try to complete a step we're not in
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "merge".to_string(), // We're in init, not merge
            exit_code: 0,
        })
        .await
        .unwrap();

    // Should still be in init
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "init");
}

/// Runbook without explicit on_done - step should complete the pipeline
const RUNBOOK_NO_ON_DONE: &str = r#"
[command.simple]
args = "<name>"
run = { pipeline = "simple" }

[pipeline.simple]
input  = ["name"]

[[pipeline.simple.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn step_without_on_done_completes_pipeline() {
    let ctx = setup_with_runbook(RUNBOOK_NO_ON_DONE).await;

    // Create pipeline
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();

    // Complete init - no on_done means pipeline should complete, not advance sequentially
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
    assert_eq!(pipeline.step_status, StepStatus::Completed);
}

/// Runbook with explicit next step transitions
const RUNBOOK_EXPLICIT_NEXT: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "init"
run = "echo init"
on_done = "custom"

[[pipeline.build.step]]
name = "custom"
run = "echo custom"
on_done = "done"

[[pipeline.build.step]]
name = "done"
run = "echo done"
"#;

#[tokio::test]
async fn explicit_next_step_is_followed() {
    let ctx = setup_with_runbook(RUNBOOK_EXPLICIT_NEXT).await;

    // Create pipeline
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();

    // Complete init - should go to custom (not second step in order)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "custom");

    // Complete custom - should go to done (from explicit next)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "custom".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
}

/// Runbook where done step has no run command (implicit completion)
const RUNBOOK_IMPLICIT_DONE: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "init"
run = "echo init"
on_done = "done"

[[pipeline.build.step]]
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();

    // Complete init - should advance to done step
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
    assert_eq!(pipeline.step_status, StepStatus::Running);

    // Complete done step - pipeline should complete
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "done".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
    assert_eq!(pipeline.step_status, StepStatus::Completed);
}

#[tokio::test]
async fn step_runs_with_fallback_workspace_path() {
    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "init");

    // Shell completion should work even if workspace_path might not be set
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");
}

#[tokio::test]
async fn advance_pipeline_cancels_exit_deferred_timer() {
    use oj_core::TimerId;
    use std::time::Duration;

    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to plan step (agent)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");

    // Manually schedule an exit-deferred timer (simulates liveness detecting death)
    {
        let scheduler = ctx.runtime.executor.scheduler();
        let mut sched = scheduler.lock();
        sched.set_timer(
            TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())).to_string(),
            Duration::from_secs(5),
            ctx.clock.now(),
        );
    }

    // Advance pipeline past the agent step
    ctx.runtime.advance_pipeline(&pipeline).await.unwrap();

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
        !timer_ids
            .contains(&TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())).as_str()),
        "advance_pipeline must cancel exit-deferred timer"
    );
}

#[tokio::test]
async fn fail_pipeline_cancels_exit_deferred_timer() {
    use oj_core::TimerId;
    use std::time::Duration;

    let ctx = setup().await;
    let pipeline_id = create_pipeline(&ctx).await;

    // Advance to plan step (agent)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "plan");

    // Manually schedule an exit-deferred timer (simulates liveness detecting death)
    {
        let scheduler = ctx.runtime.executor.scheduler();
        let mut sched = scheduler.lock();
        sched.set_timer(
            TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())).to_string(),
            Duration::from_secs(5),
            ctx.clock.now(),
        );
    }

    // Fail the pipeline from the agent step
    ctx.runtime
        .fail_pipeline(&pipeline, "test failure")
        .await
        .unwrap();

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
        !timer_ids
            .contains(&TimerId::exit_deferred(&PipelineId::new(pipeline_id.clone())).as_str()),
        "fail_pipeline must cancel exit-deferred timer"
    );
    assert!(
        !timer_ids.contains(&TimerId::liveness(&PipelineId::new(pipeline_id.clone())).as_str()),
        "fail_pipeline must cancel liveness timer"
    );
}

// --- Pipeline-level lifecycle hook tests ---

/// Runbook with pipeline-level on_done
const RUNBOOK_PIPELINE_ON_DONE: &str = r#"
[command.deploy]
args = "<name>"
run = { pipeline = "deploy" }

[pipeline.deploy]
input  = ["name"]
on_done = "teardown"

[[pipeline.deploy.step]]
name = "init"
run = "echo init"
on_done = "work"

[[pipeline.deploy.step]]
name = "work"
run = "echo work"

[[pipeline.deploy.step]]
name = "teardown"
run = "echo teardown"
"#;

#[tokio::test]
async fn pipeline_on_done_routes_to_teardown() {
    let ctx = setup_with_runbook(RUNBOOK_PIPELINE_ON_DONE).await;

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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();

    // Complete init -> work
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "work");

    // Complete work (no step-level on_done) -> should go to teardown via pipeline on_done
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "work".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(
        pipeline.step, "teardown",
        "Expected pipeline on_done to route to teardown"
    );

    // Complete teardown (also no step-level on_done, but IS the on_done target) -> should complete
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "teardown".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
    assert_eq!(pipeline.step_status, StepStatus::Completed);
}

/// Runbook with pipeline-level on_fail
const RUNBOOK_PIPELINE_ON_FAIL: &str = r#"
[command.deploy]
args = "<name>"
run = { pipeline = "deploy" }

[pipeline.deploy]
input  = ["name"]
on_fail = "cleanup"

[[pipeline.deploy.step]]
name = "init"
run = "echo init"

[[pipeline.deploy.step]]
name = "cleanup"
run = "echo cleanup"
"#;

#[tokio::test]
async fn pipeline_on_fail_routes_to_cleanup() {
    let ctx = setup_with_runbook(RUNBOOK_PIPELINE_ON_FAIL).await;

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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();

    // Fail init (no step-level on_fail) -> should go to cleanup via pipeline on_fail
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(
        pipeline.step, "cleanup",
        "Expected pipeline on_fail to route to cleanup"
    );
}

/// Runbook where step-level on_done overrides pipeline-level on_done
const RUNBOOK_STEP_ON_DONE_PRECEDENCE: &str = r#"
[command.deploy]
args = "<name>"
run = { pipeline = "deploy" }

[pipeline.deploy]
input  = ["name"]
on_done = "teardown"

[[pipeline.deploy.step]]
name = "init"
run = "echo init"
on_done = "custom"

[[pipeline.deploy.step]]
name = "custom"
run = "echo custom"

[[pipeline.deploy.step]]
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();

    // Complete init - step-level on_done = "custom" should take priority over pipeline on_done = "teardown"
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(
        pipeline.step, "custom",
        "Step-level on_done should take precedence over pipeline-level"
    );
}

/// Runbook where step-level on_fail overrides pipeline-level on_fail
const RUNBOOK_STEP_ON_FAIL_PRECEDENCE: &str = r#"
[command.deploy]
args = "<name>"
run = { pipeline = "deploy" }

[pipeline.deploy]
input  = ["name"]
on_fail = "global-cleanup"

[[pipeline.deploy.step]]
name = "init"
run = "echo init"
on_fail = "step-cleanup"

[[pipeline.deploy.step]]
name = "step-cleanup"
run = "echo step-cleanup"

[[pipeline.deploy.step]]
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();

    // Fail init - step-level on_fail = "step-cleanup" should take priority
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "init".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(
        pipeline.step, "step-cleanup",
        "Step-level on_fail should take precedence over pipeline-level"
    );
}

// --- Locals interpolation tests ---

/// Runbook with locals that reference pipeline vars via ${var.*}
const RUNBOOK_WITH_LOCALS: &str = r#"
[command.build]
args = "<name> <instructions>"
run = { pipeline = "build" }

[pipeline.build]
input = ["name", "instructions"]

[pipeline.build.locals]
branch = "feature/${var.name}"
title = "feat(${var.name}): ${var.instructions}"

[[pipeline.build.step]]
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();

    assert_eq!(
        pipeline.vars.get("local.branch").map(String::as_str),
        Some("feature/auth"),
        "local.branch should interpolate ${{var.name}}"
    );
    assert_eq!(
        pipeline.vars.get("local.title").map(String::as_str),
        Some("feat(auth): add login"),
        "local.title should interpolate ${{var.name}} and ${{var.instructions}}"
    );
}

/// Runbook with locals that reference workspace variables
const RUNBOOK_LOCALS_WITH_WORKSPACE: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input = ["name"]
workspace = "ephemeral"

[pipeline.build.locals]
branch = "feature/${var.name}-${workspace.nonce}"

[[pipeline.build.step]]
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();

    let branch = pipeline
        .vars
        .get("local.branch")
        .cloned()
        .unwrap_or_default();
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

/// Locals containing shell expressions $(...) are eagerly evaluated at pipeline
/// creation time. The output of the shell command is stored as plain data.
const RUNBOOK_LOCALS_SHELL_SUBST: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input = ["name"]

[pipeline.build.locals]
repo = "$(echo /some/repo)"

[[pipeline.build.step]]
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();

    // After eager evaluation, $(echo /some/repo) should be resolved
    assert_eq!(
        pipeline.vars.get("local.repo").map(String::as_str),
        Some("/some/repo"),
        "Shell command substitution should be eagerly evaluated in locals"
    );
}

// --- on_fail cycle tests ---

/// Runbook with on_fail self-cycle: step retries itself on failure
const RUNBOOK_ON_FAIL_SELF_CYCLE: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input = ["name"]

[[pipeline.build.step]]
name = "work"
run = "false"
on_fail = "work"
on_done = "done"

[[pipeline.build.step]]
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();

    // Set some action_attempts to simulate agent retry tracking
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.pipelines.get_mut(&pipeline_id) {
            p.increment_action_attempt("exit", 0);
            p.increment_action_attempt("exit", 0);
        }
    });

    // Verify attempts are set
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.get_action_attempt("exit", 0), 2);

    // Shell fails → on_fail = "work" (self-cycle)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "work".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "work", "should cycle back to work step");
    // action_attempts should be preserved across the on_fail cycle
    assert_eq!(
        pipeline.get_action_attempt("exit", 0),
        2,
        "action_attempts must be preserved on on_fail self-cycle"
    );
}

/// Runbook with multi-step on_fail cycle: A fails→B, B fails→A
const RUNBOOK_ON_FAIL_MULTI_STEP_CYCLE: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input = ["name"]

[[pipeline.build.step]]
name = "work"
run = "false"
on_fail = "recover"
on_done = "done"

[[pipeline.build.step]]
name = "recover"
run = "false"
on_fail = "work"

[[pipeline.build.step]]
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    assert_eq!(ctx.runtime.get_pipeline(&pipeline_id).unwrap().step, "work");

    // Set action_attempts to simulate prior attempts
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.pipelines.get_mut(&pipeline_id) {
            p.increment_action_attempt("exit", 0);
        }
    });

    // work fails → on_fail → recover
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "work".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "recover");
    assert_eq!(
        pipeline.get_action_attempt("exit", 0),
        1,
        "action_attempts preserved after work→recover on_fail transition"
    );

    // recover fails → on_fail → work (completing the cycle)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "recover".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "work");
    assert_eq!(
        pipeline.get_action_attempt("exit", 0),
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();

    // Set action_attempts
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.pipelines.get_mut(&pipeline_id) {
            p.increment_action_attempt("exit", 0);
            p.increment_action_attempt("exit", 0);
        }
    });

    // Shell succeeds → on_done = "done"
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "work".to_string(),
            exit_code: 0,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "done");
    // action_attempts should be reset on success transition
    assert_eq!(
        pipeline.get_action_attempt("exit", 0),
        0,
        "action_attempts must be reset on on_done transition"
    );
}

// --- Circuit breaker tests ---

/// Runbook with a cycle: work fails→retry, retry fails→work
const RUNBOOK_CYCLE_CIRCUIT_BREAKER: &str = r#"
[command.build]
args = "<name>"
run = { pipeline = "build" }

[pipeline.build]
input = ["name"]

[[pipeline.build.step]]
name = "work"
run = "false"
on_fail = "retry"
on_done = "done"

[[pipeline.build.step]]
name = "retry"
run = "false"
on_fail = "work"

[[pipeline.build.step]]
name = "done"
run = "echo done"
"#;

#[tokio::test]
async fn circuit_breaker_fails_pipeline_after_max_step_visits() {
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();
    assert_eq!(ctx.runtime.get_pipeline(&pipeline_id).unwrap().step, "work");

    // Drive the cycle: work→retry→work→retry→... until circuit breaker fires.
    // Each full cycle visits both "work" and "retry" once.
    // MAX_STEP_VISITS = 5, so after 5 visits to "work" the 6th should be blocked.
    // Initial visit to "work" doesn't count (it's the initial step, before PipelineAdvanced).
    // Cycle: work(fail) → retry(visit 1) → retry(fail) → work(visit 1) → ...
    let max = oj_core::pipeline::MAX_STEP_VISITS;
    for i in 0..50 {
        let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
        if pipeline.is_terminal() {
            // Circuit breaker should fire well before 50 iterations
            assert!(
                i <= (max as usize + 1) * 2,
                "circuit breaker should have fired by now (iteration {i})"
            );
            assert_eq!(pipeline.step, "failed");
            assert!(
                pipeline
                    .error
                    .as_deref()
                    .unwrap_or("")
                    .contains("circuit breaker"),
                "error should mention circuit breaker, got: {:?}",
                pipeline.error
            );
            return;
        }

        let step = pipeline.step.clone();
        ctx.runtime
            .handle_event(Event::ShellExited {
                pipeline_id: PipelineId::new(pipeline_id.clone()),
                step,
                exit_code: 1,
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

    let pipeline_id = ctx.runtime.pipelines().keys().next().unwrap().clone();

    // Initial step "work" - step_visits not yet tracked (initial step)
    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.get_step_visits("work"), 0);

    // work fails → retry (visit 1 for retry)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "work".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "retry");
    assert_eq!(pipeline.get_step_visits("retry"), 1);

    // retry fails → work (visit 1 for work via PipelineAdvanced)
    ctx.runtime
        .handle_event(Event::ShellExited {
            pipeline_id: PipelineId::new(pipeline_id.clone()),
            step: "retry".to_string(),
            exit_code: 1,
        })
        .await
        .unwrap();

    let pipeline = ctx.runtime.get_pipeline(&pipeline_id).unwrap();
    assert_eq!(pipeline.step, "work");
    assert_eq!(pipeline.get_step_visits("work"), 1);
    assert_eq!(pipeline.get_step_visits("retry"), 1);
}
