// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use oj_runbook::parse_runbook;

#[test]
fn hcl_cron_valid() {
    let hcl = r#"
job "cleanup" {
  step "run" { run = "echo cleanup" }
}
cron "janitor" {
  interval = "30m"
  run      = { job = "cleanup" }
}
"#;
    let cron = &super::parse_hcl(hcl).crons["janitor"];
    assert_eq!(cron.name, "janitor");
    assert_eq!(cron.interval, "30m");
    assert_eq!(cron.run.job_name(), Some("cleanup"));
}

#[test]
fn toml_cron_valid() {
    let toml = r#"
[job.deploy]
[[job.deploy.step]]
name = "run"
run = "echo deploy"

[cron.nightly]
interval = "24h"
run = { job = "deploy" }
"#;
    let cron = &parse_runbook(toml).unwrap().crons["nightly"];
    assert_eq!(cron.name, "nightly");
    assert_eq!(cron.interval, "24h");
    assert_eq!(cron.run.job_name(), Some("deploy"));
}

#[test]
fn error_cron_invalid_interval() {
    let hcl = r#"
job "cleanup" {
  step "run" {
    run = "echo cleanup"
  }
}
cron "janitor" {
  interval = "invalid"
  run      = { job = "cleanup" }
}
"#;
    super::assert_hcl_err(hcl, &["cron.janitor.interval"]);
}

#[test]
fn error_cron_non_job_run() {
    super::assert_hcl_err(
        "cron \"janitor\" {\n  interval = \"30m\"\n  run = \"echo cleanup\"\n}",
        &["cron run must reference a job or agent"],
    );
}

#[test]
fn hcl_cron_agent_valid() {
    let hcl = r#"
agent "doctor" {
  run     = "claude --model sonnet"
  on_idle = "done"
  prompt  = "Run diagnostics..."
}
cron "health_check" {
  interval = "30m"
  run      = { agent = "doctor" }
}
"#;
    let cron = &super::parse_hcl(hcl).crons["health_check"];
    assert_eq!(cron.interval, "30m");
    assert_eq!(cron.run.agent_name(), Some("doctor"));
}

#[test]
fn error_cron_unknown_agent() {
    super::assert_hcl_err(
        "cron \"h\" {\n  interval = \"30m\"\n  run = { agent = \"nonexistent\" }\n}",
        &["references unknown agent 'nonexistent'"],
    );
}

#[test]
fn error_cron_unknown_job() {
    super::assert_hcl_err(
        "cron \"j\" {\n  interval = \"30m\"\n  run = { job = \"nonexistent\" }\n}",
        &["references unknown job 'nonexistent'"],
    );
}

// ============================================================================
// Agent Max Concurrency (co-located with cron since cron agents use it)
// ============================================================================

#[test]
fn agent_max_concurrency() {
    let hcl = r#"
agent "doctor" {
  run             = "claude --model sonnet"
  on_idle         = "done"
  max_concurrency = 1
  prompt          = "Run diagnostics..."
}
"#;
    assert_eq!(
        super::parse_hcl(hcl).agents["doctor"].max_concurrency,
        Some(1)
    );
}

#[test]
fn agent_max_concurrency_default() {
    let hcl = "agent \"doctor\" {\n  run = \"claude --model sonnet\"\n  on_idle = \"done\"\n  prompt = \"Run diagnostics...\"\n}";
    assert_eq!(super::parse_hcl(hcl).agents["doctor"].max_concurrency, None);
}

#[test]
fn error_agent_max_concurrency_zero() {
    let hcl = r#"
agent "doctor" {
  run             = "claude --model sonnet"
  on_idle         = "done"
  max_concurrency = 0
  prompt          = "Run diagnostics..."
}
"#;
    super::assert_hcl_err(hcl, &["max_concurrency must be >= 1"]);
}
