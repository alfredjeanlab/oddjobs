// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runtime tests

mod crew;
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
use crate::engine::test_helpers::{
    agent_exited, agent_waiting, assert_no_timer_with_prefix, setup_with_runbook, shell_fail,
    shell_ok, vars, worker_started, TestContext,
};
use oj_core::{AgentId, CrewId, JobId};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

fn command_event(
    job_id: &str,
    job_name: &str,
    command: &str,
    args: HashMap<String, String>,
    project_path: &Path,
) -> Event {
    Event::CommandRun {
        owner: JobId::from_string(job_id).into(),
        name: job_name.to_string(),
        project_path: project_path.to_path_buf(),
        invoke_dir: project_path.to_path_buf(),
        command: command.to_string(),
        project: String::new(),
        args,
    }
}

fn crew_command_event(
    crew_id: &str,
    agent_name: &str,
    command: &str,
    args: HashMap<String, String>,
    project_path: &Path,
) -> Event {
    Event::CommandRun {
        owner: CrewId::from_string(crew_id).into(),
        name: agent_name.to_string(),
        project_path: project_path.to_path_buf(),
        invoke_dir: project_path.to_path_buf(),
        command: command.to_string(),
        project: String::new(),
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

/// Job with agent step: `command.build → job.build → [agent_step(, shell_step)]`.
///
/// `lifecycle` is appended verbatim inside `[agent.worker]` — include `run`, `prompt`,
/// `on_idle`, `on_dead`, etc. as TOML key-value lines.
fn test_runbook(step: &str, next: &str, lifecycle: &str) -> String {
    let next_block = if next.is_empty() {
        String::new()
    } else {
        format!("on_done = {{ step = \"{next}\" }}\n\n[[job.build.step]]\nname = \"{next}\"\nrun = \"echo done\"\n\n")
    };
    format!(
        "\n[command.build]\nargs = \"<name>\"\nrun = {{ job = \"build\" }}\n\n\
         [job.build]\ninput = [\"name\"]\n\n\
         [[job.build.step]]\nname = \"{step}\"\nrun = {{ agent = \"worker\" }}\n\
         {next_block}\
         [agent.worker]\n{lifecycle}\n"
    )
}

/// Simple shell job: `command.{name} → job.{name} → step "init"`.
fn test_runbook_shell(name: &str, job_config: &str) -> String {
    format!(
        "\n[command.{name}]\nargs = \"<name>\"\nrun = {{ job = \"{name}\" }}\n\n\
         [job.{name}]\ninput = [\"name\"]\n{job_config}\n\n\
         [[job.{name}.step]]\nname = \"init\"\nrun = \"echo init\"\n"
    )
}

/// Worker with queue: `job.build + queue.bugs + worker.fixer`.
fn test_runbook_worker(queue_config: &str, concurrency: u32) -> String {
    format!(
        "\n[command.build]\nargs = \"<name>\"\nrun = {{ job = \"build\" }}\n\n\
         [job.build]\ninput = [\"name\"]\n\n\
         [[job.build.step]]\nname = \"init\"\nrun = \"echo init\"\non_done = {{ step = \"done\" }}\n\n\
         [[job.build.step]]\nname = \"done\"\nrun = \"echo done\"\n\n\
         [queue.bugs]\n{queue_config}\n\n\
         [worker.fixer]\nsource = {{ queue = \"bugs\" }}\n\
         run = {{ job = \"build\" }}\nconcurrency = {concurrency}\n"
    )
}

/// Standalone agent command: `command.agent_cmd → agent.worker`.
fn test_runbook_agent(lifecycle: &str) -> String {
    format!(
        "\n[command.agent_cmd]\nargs = \"<name>\"\nrun = {{ agent = \"worker\" }}\n\n\
         [agent.worker]\nrun = \"claude\"\nprompt = \"Do the work\"\n{lifecycle}\n"
    )
}

/// Multi-step command job: `command.{name} → job.{name} → [shell steps]`.
///
/// Steps are `(name, run, extra_config)` tuples. If `run` starts with `{` it is
/// emitted verbatim (inline table); otherwise it is quoted as a string.
fn test_runbook_steps(name: &str, job_config: &str, steps: &[(&str, &str, &str)]) -> String {
    let input = if job_config.contains("input") {
        String::new()
    } else {
        "input = [\"name\"]\n".to_string()
    };
    let jc = if job_config.is_empty() { String::new() } else { format!("{job_config}\n") };
    let mut s = format!("\n[command.{name}]\nargs = \"<name>\"\nrun = {{ job = \"{name}\" }}\n\n[job.{name}]\n{input}{jc}\n");
    for &(sn, run, cfg) in steps {
        let rv = if run.starts_with('{') { run.to_string() } else { format!("\"{run}\"") };
        let c = if cfg.is_empty() { String::new() } else { format!("{cfg}\n") };
        s.push_str(&format!("[[job.{name}.step]]\nname = \"{sn}\"\nrun = {rv}\n{c}\n"));
    }
    s
}

/// Cron-triggered job: `cron.{cron} → job.{job} → [shell steps]`.
fn test_runbook_cron_job(
    cron: &str,
    job: &str,
    cron_cfg: &str,
    steps: &[(&str, &str, &str)],
) -> String {
    let mut s =
        format!("\n[cron.{cron}]\n{cron_cfg}\nrun = {{ job = \"{job}\" }}\n\n[job.{job}]\n\n");
    for &(sn, run, cfg) in steps {
        let c = if cfg.is_empty() { String::new() } else { format!("{cfg}\n") };
        s.push_str(&format!("[[job.{job}.step]]\nname = \"{sn}\"\nrun = \"{run}\"\n{c}\n"));
    }
    s
}

/// Cron-triggered agent: `cron.health_check → agent.doctor`.
fn test_runbook_cron_agent(cron_cfg: &str, agent_cfg: &str) -> String {
    let ac = if agent_cfg.is_empty() { String::new() } else { format!("{agent_cfg}\n") };
    format!("\n[cron.health_check]\n{cron_cfg}\nrun = {{ agent = \"doctor\" }}\n\n[agent.doctor]\n{ac}run = \"claude --print\"\nprompt = \"Run diagnostics\"\n")
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
on_done = { step = "plan" }

[[job.build.step]]
name = "plan"
run = { agent = "planner" }
on_done = { step = "execute" }

[[job.build.step]]
name = "execute"
run = { agent = "executor" }
on_done = { step = "merge" }

[[job.build.step]]
name = "merge"
run = "echo merge"
on_done = { step = "done" }
on_fail = { step = "cleanup" }

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
    create_job_with_id(ctx, "job-1").await
}

/// Get the agent_id for a job's current step from step history.
fn get_agent_id(ctx: &TestContext, job_id: &str) -> Option<AgentId> {
    let job = ctx.runtime.get_job(job_id)?;
    job.step_history
        .iter()
        .rfind(|r| r.name == job.step)
        .and_then(|r| r.agent_id.clone())
        .map(AgentId::from_string)
}

async fn create_job_with_id(ctx: &TestContext, job_id: &str) -> String {
    let args = vars!("name" => "test-feature", "prompt" => "Add login");

    handle_event_chain(ctx, command_event(job_id, "build", "build", args, &ctx.project_path)).await;

    job_id.to_string()
}

/// Run a command via `handle_event_chain` and return the job ID (if any).
///
/// Uses "job-1" as job_id, the command name as job_name, and merges any
/// additional `args` into the default `{"name": "test"}` vars.
///
/// For commands that create a job (`run = { job = "..." }`), returns the job ID.
/// For standalone agent commands (`run = { agent = "..." }`), returns `"job-1"`.
async fn create_job_for_runbook(ctx: &TestContext, command: &str, args: &[(&str, &str)]) -> String {
    let mut all_args = vars!("name" => "test");
    for &(k, v) in args {
        all_args.insert(k.to_string(), v.to_string());
    }

    // Detect if this command targets an agent (crew) by loading the runbook
    let runbook_dir = ctx.project_path.join(".oj/runbooks");
    let is_agent = oj_runbook::find_runbook_by_command(&runbook_dir, command)
        .ok()
        .flatten()
        .and_then(|rb| rb.get_command(command).cloned())
        .is_some_and(|cmd| matches!(cmd.run, oj_runbook::RunDirective::Agent { .. }));

    let event = if is_agent {
        crew_command_event("job-1", command, command, all_args, &ctx.project_path)
    } else {
        command_event("job-1", command, command, all_args, &ctx.project_path)
    };
    handle_event_chain(ctx, event).await;
    ctx.runtime.jobs().keys().next().cloned().unwrap_or_else(|| "job-1".to_string())
}

/// Helper: parse a runbook string, serialize, and return (json_value, sha256_hash).
/// Used by cron and worker tests.
pub(super) fn hash_runbook(content: &str) -> (serde_json::Value, String) {
    let runbook = oj_runbook::parse_runbook(content).unwrap();
    let runbook_json = serde_json::to_value(&runbook).unwrap();
    let runbook_hash = {
        let canonical = serde_json::to_string(&runbook_json).unwrap();
        let digest = Sha256::digest(canonical.as_bytes());
        format!("{:x}", digest)
    };
    (runbook_json, runbook_hash)
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
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");
}

#[tokio::test]
async fn agent_done_advances_step() {
    let ctx = setup().await;
    let job_id = create_job(&ctx).await;

    // Advance to plan step (agent)
    ctx.runtime.handle_event(shell_ok(&job_id, "init")).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "plan");

    // Advance job (orchestrator-driven)
    ctx.runtime.advance_job(&job).await.unwrap();

    let job = ctx.runtime.get_job(&job_id).unwrap();
    assert_eq!(job.step, "execute");
}
