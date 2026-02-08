// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runtime tests

mod agent_run;
mod cron;
mod cron_agent;
mod cron_concurrency;
mod directives;
mod errors;
mod idempotency;
mod job_create;
mod job_deleted;
mod monitoring;
mod notify;
mod on_dead;
mod resume;
mod sessions;
mod steps;
mod steps_cycles;
mod steps_lifecycle;
mod steps_locals;
mod timer_cleanup;
mod worker;
mod worker_concurrency;
mod worker_external;
mod worker_queue;

use super::*;
use crate::test_helpers::{setup_with_runbook, TestContext};
use oj_core::{AgentId, JobId};
use std::collections::HashMap;
use std::path::Path;

fn command_event(
    job_id: &str,
    job_name: &str,
    command: &str,
    args: HashMap<String, String>,
    project_root: &Path,
) -> Event {
    Event::CommandRun {
        job_id: JobId::new(job_id),
        job_name: job_name.to_string(),
        project_root: project_root.to_path_buf(),
        invoke_dir: project_root.to_path_buf(),
        command: command.to_string(),
        namespace: String::new(),
        args,
    }
}

/// Process an event and all cascading result events until stable.
///
/// Simulates the daemon event loop: result events from `handle_event` are
/// re-processed (e.g., `CommandRun` → `JobCreated` → workspace/step start).
async fn handle_event_chain(ctx: &TestContext, event: Event) {
    let mut queue = vec![event];
    while let Some(event) = queue.pop() {
        let result = ctx.runtime.handle_event(event).await.unwrap();
        queue.extend(result);
    }
}

const TEST_RUNBOOK: &str = r#"
[command.build]
args = "<name> <prompt>"
run = { job = "build" }

[job.build]
input  = ["name", "prompt"]

[[job.build.step]]
name = "init"
run = "echo init"
on_done = "plan"

[[job.build.step]]
name = "plan"
run = { agent = "planner" }
on_done = "execute"

[[job.build.step]]
name = "execute"
run = { agent = "executor" }
on_done = "merge"

[[job.build.step]]
name = "merge"
run = "echo merge"
on_done = "done"
on_fail = "cleanup"

[[job.build.step]]
name = "done"
run = "echo done"

[[job.build.step]]
name = "cleanup"
run = "echo cleanup"

[agent.planner]
run = "claude --print"
[agent.planner.env]
OJ_STEP = "plan"

[agent.executor]
run = "claude --print"
[agent.executor.env]
OJ_STEP = "execute"
"#;

async fn setup() -> TestContext {
    setup_with_runbook(TEST_RUNBOOK).await
}

async fn create_job(ctx: &TestContext) -> String {
    create_job_with_id(ctx, "pipe-1").await
}

/// Get the agent_id for a job's current step from step history.
fn get_agent_id(ctx: &TestContext, job_id: &str) -> Option<AgentId> {
    let job = ctx.runtime.get_job(job_id)?;
    job.step_history
        .iter()
        .rfind(|r| r.name == job.step)
        .and_then(|r| r.agent_id.clone())
        .map(AgentId::new)
}

async fn create_job_with_id(ctx: &TestContext, job_id: &str) -> String {
    let args: HashMap<String, String> = [
        ("name".to_string(), "test-feature".to_string()),
        ("prompt".to_string(), "Add login".to_string()),
    ]
    .into_iter()
    .collect();

    handle_event_chain(
        ctx,
        command_event(job_id, "build", "build", args, &ctx.project_root),
    )
    .await;

    job_id.to_string()
}

/// Helper: parse a runbook string, serialize, and return (json_value, sha256_hash).
/// Used by cron and worker tests.
pub(super) fn hash_runbook(content: &str) -> (serde_json::Value, String) {
    let runbook = oj_runbook::parse_runbook(content).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let runbook_hash = {
        use sha2::{Digest, Sha256};
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };
    (runbook_json, runbook_hash)
}

/// Collect all pending timer IDs from the scheduler by advancing time far
/// into the future and draining fired timers.
pub(super) fn pending_timer_ids(ctx: &TestContext) -> Vec<String> {
    let scheduler = ctx.runtime.executor.scheduler();
    let mut sched = scheduler.lock();
    ctx.clock.advance(std::time::Duration::from_secs(7200));
    let fired = sched.fired_timers(ctx.clock.now());
    fired
        .into_iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str().to_string()),
            _ => None,
        })
        .collect()
}

/// Helper: check that no job-scoped timer with the given prefix exists.
pub(super) fn assert_no_timer_with_prefix(timer_ids: &[String], prefix: &str) {
    let matching: Vec<&String> = timer_ids
        .iter()
        .filter(|id| id.starts_with(prefix))
        .collect();
    assert!(
        matching.is_empty(),
        "expected no timers starting with '{}', found: {:?}",
        prefix,
        matching
    );
}

#[tokio::test]
async fn runtime_handle_command() {
    let ctx = setup().await;
    let _job_id = create_job(&ctx).await;

    let jobs = ctx.runtime.jobs();
    assert_eq!(jobs.len(), 1);

    let job = jobs.values().next().unwrap();
    assert_eq!(job.name, "test-feature");
    assert_eq!(job.kind, "build");
}

#[tokio::test]
async fn shell_completion_advances_step() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Job starts at init step (shell)
    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "init");

    // Simulate shell completion
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
async fn agent_done_advances_step() {
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

    // Advance job (orchestrator-driven)
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "execute");
}
