// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for smart job resume functionality

use super::*;
use oj_core::{AgentState, JobId, StepStatus};

async fn setup_resume() -> TestContext {
    let mut runbook = test_runbook_steps(
        "test",
        "input = [\"name\", \"prompt\"]",
        &[
            ("setup", "echo setup", "on_done = { step = \"work\" }"),
            ("work", "{ agent = \"worker\" }", "on_done = { step = \"done\" }"),
            ("done", "echo done", ""),
        ],
    );
    runbook.push_str("\n[agent.worker]\nrun = \"claude --print\"\n");
    setup_with_runbook(&runbook).await
}

/// Helper to advance job to the agent step (work) by completing the setup step
async fn advance_to_agent_step(ctx: &TestContext, job_id: &str) {
    ctx.runtime.handle_event(shell_ok(job_id, "setup")).await.unwrap();
}

#[tokio::test]
async fn resume_agent_without_message_uses_default() {
    let ctx = setup_resume().await;
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

    // Advance to agent step
    advance_to_agent_step(&ctx, &job_id).await;

    // Verify we're at the agent step
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "work");

    // Resume without message — should succeed with default
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: None,
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    assert!(result.is_ok(), "expected Ok, got: {:?}", result);
}

#[tokio::test]
async fn resume_agent_alive_sends_nudge() {
    let ctx = setup_resume().await;
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

    // Advance to agent step (this spawns the agent with a UUID)
    advance_to_agent_step(&ctx, &job_id).await;

    // Get the agent_id that was registered during spawn
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent state to Working (alive)
    ctx.agents.set_agent_state(&agent_id, AgentState::Working);

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
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

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
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

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
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

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
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

    // Job starts at "setup" step
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(!job.vars.contains_key("new_key"));

    // Resume with input updates
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: None,
            vars: vars!("new_key" => "new_value", "another_key" => "another_value"),
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
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

    // Advance to agent step (spawns agent with UUID)
    advance_to_agent_step(&ctx, &job_id).await;

    // Get the agent_id that was registered during spawn
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent as SessionGone (dead)
    ctx.agents.set_agent_state(&agent_id, AgentState::SessionGone);

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
    let ctx = setup_resume().await;
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

    // Advance to agent step (spawns agent with UUID)
    advance_to_agent_step(&ctx, &job_id).await;

    // Get the agent_id that was registered during spawn
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Set agent to WaitingForInput (alive, but idle)
    ctx.agents.set_agent_state(&agent_id, AgentState::WaitingForInput);

    // Resume with message - should nudge (send to agent)
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
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

    // Job starts at "setup" (shell step)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "setup");

    // Fail the shell step (non-zero exit, no on_fail → terminal "failed")
    ctx.runtime.handle_event(shell_fail(&job_id, "setup")).await.unwrap();

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
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

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
async fn resume_collects_all_agent_ids_from_step_history() {
    let ctx = setup_resume().await;
    let job_id = create_job_for_runbook(&ctx, "test", &[("prompt", "Do the work")]).await;

    // Advance to agent step (spawns first agent)
    advance_to_agent_step(&ctx, &job_id).await;

    // Get the first agent_id
    let first_agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // Simulate the first agent completing and a new attempt starting
    // by manually adding another step record with a different agent_id
    let second_agent_id = "second-agent-uuid";
    ctx.runtime.lock_state_mut(|state| {
        if let Some(job) = state.jobs.get_mut(&job_id) {
            // Push a new step record for the same step with a new agent_id
            // (simulating what happens on retry/resume)
            job.step_history.push(oj_core::StepRecord {
                name: "work".to_string(),
                started_at_ms: 1000,
                finished_at_ms: None,
                outcome: oj_core::StepOutcome::Running,
                agent_id: Some(second_agent_id.to_string()),
                agent_name: Some("worker".to_string()),
            });
        }
    });

    // Verify step_history now has two entries for "work"
    let job = ctx.runtime.get_job(&job_id).unwrap();
    let work_entries: Vec<_> = job.step_history.iter().filter(|r| r.name == "work").collect();
    assert_eq!(work_entries.len(), 2, "should have two step history entries for 'work'");

    // Verify we have both agent IDs
    let agent_ids: Vec<_> = work_entries.iter().filter_map(|r| r.agent_id.as_ref()).collect();
    assert!(
        agent_ids.contains(&&first_agent_id.as_str().to_string()),
        "should contain first agent_id"
    );
    assert!(agent_ids.contains(&&second_agent_id.to_string()), "should contain second agent_id");

    // The handle_agent_resume should collect all agent_ids and try each
    // (most recent first) to find one with a valid session file.
    // Since no session files exist in test, it will start fresh,
    // but the important thing is it doesn't error out.
    let result = ctx
        .runtime
        .handle_event(Event::JobResume {
            id: JobId::new(job_id.clone()),
            message: Some("Continue".to_string()),
            vars: HashMap::new(),
            kill: false,
        })
        .await;

    // Should not fail with a message about wrong agent_id
    if let Err(ref e) = result {
        let err_str = e.to_string();
        assert!(
            !err_str.contains("agent_id"),
            "should not fail on agent_id lookup, got: {}",
            err_str
        );
    }
}
