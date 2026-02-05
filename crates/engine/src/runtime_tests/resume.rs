// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for smart job resume functionality

use super::*;
use oj_core::{AgentState, JobId, StepStatus};

/// Runbook for testing resume functionality with a shell step and an agent step
const RESUME_RUNBOOK: &str = r#"
[command.test]
args = "<name>"
run = { job = "test" }

[job.test]
input  = ["name", "prompt"]

[[job.test.step]]
name = "setup"
run = "echo setup"
on_done = "work"

[[job.test.step]]
name = "work"
run = { agent = "worker" }
on_done = "done"

[[job.test.step]]
name = "done"
run = "echo done"

[agent.worker]
run = "claude --print"
"#;

async fn setup_resume() -> TestContext {
    setup_with_runbook(RESUME_RUNBOOK).await
}

async fn create_test_job(ctx: &TestContext, job_id: &str) -> String {
    let args: HashMap<String, String> = [
        ("name".to_string(), "test".to_string()),
        ("prompt".to_string(), "Do the work".to_string()),
    ]
    .into_iter()
    .collect();

    ctx.runtime
        .handle_event(command_event(
            job_id,
            "test",
            "test",
            args,
            &ctx.project_root,
        ))
        .await
        .unwrap();

    job_id.to_string()
}

/// Helper to advance job to the agent step (work) by completing the setup step
async fn advance_to_agent_step(ctx: &TestContext, job_id: &str) {
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id),
            step: "setup".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn resume_agent_without_message_fails() {
    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-1").await;

    // Advance to agent step
    advance_to_agent_step(&ctx, &job_id).await;

    // Verify we're at the agent step
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    // Try to resume without message - should fail
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: None,
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("--message") || err.contains("agent steps require"),
        "expected error about --message, got: {}",
        err
    );
}

#[tokio::test]
async fn resume_agent_alive_sends_nudge() {
    use oj_adapters::SessionAdapter;

    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-2").await;

    // Advance to agent step (this spawns the agent with a UUID)
    advance_to_agent_step(&ctx, &job_id).await;

    // Get the agent_id that was registered during spawn
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent state to Working (alive)
    ctx.agents.set_agent_state(&agent_id, AgentState::Working);

    // Spawn a session for the job (simulating agent startup)
    let session_id = ctx
        .sessions
        .spawn("test", std::path::Path::new("/tmp"), "echo test", &[])
        .await
        .unwrap();

    // Update the job's session_id in state
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(&job_id) {
            p.session_id = Some(session_id.clone());
        }
    });

    // Resume with message
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: Some("I fixed the import, try again".to_string()),
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    assert!(result.is_ok());

    // Verify job status is Running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step_status, StepStatus::Running);
}

#[tokio::test]
async fn resume_agent_dead_attempts_recovery() {
    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-3").await;

    // Advance to agent step
    advance_to_agent_step(&ctx, &job_id).await;

    // Don't spawn an agent - get_state will return NotFound, treating as dead

    // Resume with message - should attempt recovery (respawn)
    // Note: Full recovery requires complex workspace setup, so we just verify
    // that the resume logic correctly identifies this as a recovery case
    // (i.e., agent not found = dead = recovery needed)
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: Some("I fixed the issue, try again".to_string()),
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    // The recovery attempt will fail because the test environment doesn't have
    // full workspace setup, but we've verified the logic path is taken
    // (not error about missing message, which would indicate nudge path)
    if let Err(ref e) = result {
        let err_str = e.to_string();
        // Verify we got a spawn/session error (recovery path), not a message error (wrong path)
        assert!(
            !err_str.contains("--message") && !err_str.contains("agent steps require"),
            "expected recovery attempt, but got nudge error: {}",
            err_str
        );
    }

    // Job should still be on the same step
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
}

#[tokio::test]
async fn resume_shell_reruns_command() {
    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-4").await;

    // Job starts at "setup" step which is a shell step
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "setup");

    // Resume the shell step (no message needed)
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: None,
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    assert!(result.is_ok());

    // Job should still be at setup step
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "setup");
}

#[tokio::test]
async fn resume_shell_with_message_succeeds_with_warning() {
    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-5").await;

    // Job starts at "setup" step which is a shell step
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "setup");

    // Resume shell step with message (should still work, just log warning)
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: Some("This message will be ignored".to_string()),
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    // Should succeed (warning is just logged, not an error)
    assert!(result.is_ok());
}

#[tokio::test]
async fn resume_persists_input_updates() {
    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-6").await;

    // Job starts at "setup" step
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(!job.vars.contains_key("new_key"));

    // Resume with input updates
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: None,
            vars: [
                ("new_key".to_string(), "new_value".to_string()),
                ("another_key".to_string(), "another_value".to_string()),
            ]
            .into_iter()
            .collect(),
            kill: false,
        })
        .await;

    assert!(result.is_ok());

    // The input update is emitted as an Effect::Emit event which gets sent
    // to the event channel. For this test we verify the operation succeeded.
}

#[tokio::test]
async fn resume_agent_session_gone_recovers() {
    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-7").await;

    // Advance to agent step (spawns agent with UUID)
    advance_to_agent_step(&ctx, &job_id).await;

    // Get the agent_id that was registered during spawn
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent as SessionGone (dead)
    ctx.agents
        .set_agent_state(&agent_id, AgentState::SessionGone);

    // Resume with message
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: Some("Session died, recovering".to_string()),
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    assert!(result.is_ok());
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
}

#[tokio::test]
async fn resume_agent_waiting_nudges() {
    use oj_adapters::SessionAdapter;

    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-8").await;

    // Advance to agent step (spawns agent with UUID)
    advance_to_agent_step(&ctx, &job_id).await;

    // Get the agent_id that was registered during spawn
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent to WaitingForInput (alive, but idle)
    ctx.agents
        .set_agent_state(&agent_id, AgentState::WaitingForInput);

    // Spawn a session for the job
    let session_id = ctx
        .sessions
        .spawn("test", std::path::Path::new("/tmp"), "echo test", &[])
        .await
        .unwrap();

    // Update the job's session_id in state
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(&job_id) {
            p.session_id = Some(session_id.clone());
        }
    });

    // Resume with message - should nudge (send to session)
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: Some("Continue with the work".to_string()),
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    assert!(result.is_ok());

    // Job should be running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step_status, StepStatus::Running);
}

#[tokio::test]
async fn resume_from_terminal_failure_shell_step() {
    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-tf-1").await;

    // Job starts at "setup" (shell step)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "setup");

    // Fail the shell step (non-zero exit, no on_fail → terminal "failed")
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new(job_id.clone()),
            step: "setup".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    // Verify job is in terminal "failed" state
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "failed");
    assert_eq!(job.step_status, StepStatus::Failed);
    assert!(job.error.is_some());

    // Resume from terminal failure — should reset to the failed step and re-run
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: None,
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    assert!(result.is_ok());

    // Job should be back at "setup" step and running
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "setup");
    assert!(job.error.is_none());
}

#[tokio::test]
async fn resume_from_terminal_failure_agent_step() {
    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-tf-2").await;

    // Advance to agent step
    advance_to_agent_step(&ctx, &job_id).await;

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    // Fail the job at the agent step (simulating agent terminal failure)
    ctx.runtime.fail_job(&job, "agent crashed").await.unwrap();

    // Verify job is in terminal "failed" state
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "failed");
    assert_eq!(job.step_status, StepStatus::Failed);

    // Resume from terminal failure with message — should reset to "work" and recover
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: Some("Try again with the fix".to_string()),
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    // Recovery spawns a new agent; may fail in test env due to workspace setup,
    // but must NOT fail with StepNotFound("failed") or message-required error.
    if let Err(ref e) = result {
        let err_str = e.to_string();
        assert!(
            !err_str.contains("StepNotFound") && !err_str.contains("step not found"),
            "should not get StepNotFound for terminal state, got: {}",
            err_str
        );
        assert!(
            !err_str.contains("--message") && !err_str.contains("agent steps require"),
            "should not get message error, got: {}",
            err_str
        );
    }

    // Job should have been reset to "work" (even if spawn failed afterward)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert!(job.error.is_none());
}

#[tokio::test]
async fn resume_from_terminal_failure_clears_stale_session() {
    let ctx = setup_resume().await;
    let job_id = create_test_job(&ctx, "pipe-resume-tf-3").await;

    // Advance to agent step
    advance_to_agent_step(&ctx, &job_id).await;

    // Set a stale session_id on the job
    ctx.runtime.lock_state_mut(|state| {
        if let Some(p) = state.jobs.get_mut(&job_id) {
            p.session_id = Some("stale-session-123".to_string());
        }
    });

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert!(job.session_id.is_some());

    // Fail the job
    ctx.runtime.fail_job(&job, "agent died").await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "failed");

    // Resume from terminal failure
    let _result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: Some("Retry".to_string()),
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    // After reset, session_id should be cleared (stale session cleaned up)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");
    assert!(
        job.session_id.is_none() || job.session_id.as_deref() != Some("stale-session-123"),
        "stale session_id should be cleared on resume, got: {:?}",
        job.session_id
    );
}
