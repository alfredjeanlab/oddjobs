// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for job notification lifecycle (on_start, on_done, on_fail)

use super::*;
use oj_core::OwnerId;

const NOTIFY_ON_START_RUNBOOK: &str = r#"
[command.notified]
args = "<name>"
run = { job = "notified" }

[job.notified]
input  = ["name"]
notify = { on_start = "Job ${name} started" }

[[job.notified.step]]
name = "init"
run = "echo ok"
"#;

const NOTIFY_ON_DONE_RUNBOOK: &str = r#"
[command.notified]
args = "<name>"
run = { job = "notified" }

[job.notified]
input  = ["name"]
notify = { on_done = "Job ${name} completed" }

[[job.notified.step]]
name = "init"
run = "echo ok"
"#;

const NOTIFY_ON_FAIL_RUNBOOK: &str = r#"
[command.notified]
args = "<name>"
run = { job = "notified" }

[job.notified]
input  = ["name"]
notify = { on_fail = "Job ${name} failed: ${error}" }

[[job.notified.step]]
name = "init"
run = "exit 1"
"#;

#[tokio::test]
async fn job_on_start_emits_notification() {
    let ctx = setup_with_runbook(NOTIFY_ON_START_RUNBOOK).await;

    let args: HashMap<String, String> = [("name".to_string(), "my-feature".to_string())]
        .into_iter()
        .collect();

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "notified",
            "notified",
            args,
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let calls = ctx.notifier.calls();
    assert_eq!(calls.len(), 1, "on_start should emit one notification");
    assert_eq!(calls[0].title, "my-feature");
    assert_eq!(calls[0].message, "Job my-feature started");
}

#[tokio::test]
async fn job_on_done_emits_notification() {
    let ctx = setup_with_runbook(NOTIFY_ON_DONE_RUNBOOK).await;

    let args: HashMap<String, String> = [("name".to_string(), "my-feature".to_string())]
        .into_iter()
        .collect();

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "notified",
            "notified",
            args,
            &ctx.project_root,
        ))
        .await
        .unwrap();

    // No notification yet (on_done fires on completion, not start)
    assert_eq!(ctx.notifier.calls().len(), 0);

    // Simulate shell completion
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new("pipe-1"),
            step: "init".to_string(),
            exit_code: 0,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let calls = ctx.notifier.calls();
    assert_eq!(calls.len(), 1, "on_done should emit one notification");
    assert_eq!(calls[0].title, "my-feature");
    assert_eq!(calls[0].message, "Job my-feature completed");
}

#[tokio::test]
async fn job_on_fail_emits_notification() {
    let ctx = setup_with_runbook(NOTIFY_ON_FAIL_RUNBOOK).await;

    let args: HashMap<String, String> = [("name".to_string(), "my-feature".to_string())]
        .into_iter()
        .collect();

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "notified",
            "notified",
            args,
            &ctx.project_root,
        ))
        .await
        .unwrap();

    // No notification yet
    assert_eq!(ctx.notifier.calls().len(), 0);

    // Simulate shell failure
    ctx.runtime
        .handle_event(Event::ShellExited {
            job_id: JobId::new("pipe-1"),
            step: "init".to_string(),
            exit_code: 1,
            stdout: None,
            stderr: None,
        })
        .await
        .unwrap();

    let calls = ctx.notifier.calls();
    assert_eq!(calls.len(), 1, "on_fail should emit one notification");
    assert_eq!(calls[0].title, "my-feature");
    assert!(
        calls[0].message.contains("failed"),
        "on_fail message should contain 'failed': {}",
        calls[0].message
    );
}

// =============================================================================
// Gate failure should NOT produce automatic notifications
// =============================================================================

const GATE_NO_NOTIFY_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input  = ["name"]

[[job.build.step]]
name = "work"
run = { agent = "worker" }
on_done = "done"

[[job.build.step]]
name = "done"
run = "echo done"

[agent.worker]
run = 'claude'
prompt = "Test"
on_idle = { action = "gate", run = "false" }
"#;

#[tokio::test]
async fn gate_failure_does_not_produce_automatic_notification() {
    let ctx = setup_with_runbook(GATE_NO_NOTIFY_RUNBOOK).await;

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
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // No notifications yet
    assert_eq!(ctx.notifier.calls().len(), 0);

    // Agent goes idle → on_idle gate runs "false" → gate fails → decision created
    ctx.agents
        .set_agent_state(&agent_id, oj_core::AgentState::WaitingForInput);
    ctx.runtime
        .handle_event(Event::AgentWaiting {
            agent_id: agent_id.clone(),
            owner: OwnerId::Job(JobId::new(&job_id)),
        })
        .await
        .unwrap();

    // Job should be waiting (decision created)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting());

    // No automatic notification should have fired
    let calls = ctx.notifier.calls();
    assert_eq!(
        calls.len(),
        0,
        "gate failure should not produce automatic notification, got: {:?}",
        calls
    );
}

const GATE_DEAD_NO_NOTIFY_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input  = ["name"]

[[job.build.step]]
name = "work"
run = { agent = "worker" }
on_done = "done"

[[job.build.step]]
name = "done"
run = "echo done"

[agent.worker]
run = 'claude'
prompt = "Test"
on_dead = { action = "gate", run = "false" }
"#;

#[tokio::test]
async fn gate_dead_failure_does_not_produce_automatic_notification() {
    let ctx = setup_with_runbook(GATE_DEAD_NO_NOTIFY_RUNBOOK).await;

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
    let agent_id = get_agent_id(&ctx, &job_id).unwrap();

    // No notifications yet
    assert_eq!(ctx.notifier.calls().len(), 0);

    // Agent exits → on_dead gate runs "false" → gate fails → decision created
    ctx.runtime
        .handle_event(Event::AgentExited {
            agent_id: agent_id.clone(),
            exit_code: Some(0),
            owner: OwnerId::Job(JobId::new(&job_id)),
        })
        .await
        .unwrap();

    // Job should be waiting (decision created)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert!(job.step_status.is_waiting());

    // No automatic notification should have fired
    let calls = ctx.notifier.calls();
    assert_eq!(
        calls.len(),
        0,
        "gate failure should not produce automatic notification, got: {:?}",
        calls
    );
}
