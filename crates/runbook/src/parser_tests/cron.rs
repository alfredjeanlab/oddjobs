// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::{parse_runbook, parse_runbook_with_format, Format, ParseError};

#[test]
fn parse_hcl_cron_valid() {
    let hcl = r#"
job "cleanup" {
  step "run" {
    run = "echo cleanup"
  }
}

cron "janitor" {
  interval = "30m"
  run      = { job = "cleanup" }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    assert!(runbook.crons.contains_key("janitor"));
    let cron = &runbook.crons["janitor"];
    assert_eq!(cron.name, "janitor");
    assert_eq!(cron.interval, "30m");
    assert_eq!(cron.run.job_name(), Some("cleanup"));
}

#[test]
fn parse_toml_cron_valid() {
    let toml = r#"
[job.deploy]
[[job.deploy.step]]
name = "run"
run = "echo deploy"

[cron.nightly]
interval = "24h"
run = { job = "deploy" }
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.crons.contains_key("nightly"));
    let cron = &runbook.crons["nightly"];
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
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("cron.janitor.interval"),
        "error should mention location: {}",
        msg
    );
}

#[test]
fn error_cron_non_job_run() {
    let hcl = r#"
cron "janitor" {
  interval = "30m"
  run      = "echo cleanup"
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("cron run must reference a job or agent"),
        "error should mention job/agent requirement: {}",
        msg
    );
}

#[test]
fn parse_hcl_cron_agent_valid() {
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
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    assert!(runbook.crons.contains_key("health_check"));
    let cron = &runbook.crons["health_check"];
    assert_eq!(cron.name, "health_check");
    assert_eq!(cron.interval, "30m");
    assert_eq!(cron.run.agent_name(), Some("doctor"));
}

#[test]
fn error_cron_unknown_agent() {
    let hcl = r#"
cron "health_check" {
  interval = "30m"
  run      = { agent = "nonexistent" }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown agent 'nonexistent'"),
        "error should mention unknown agent: {}",
        msg
    );
}

#[test]
fn parse_agent_max_concurrency() {
    let hcl = r#"
agent "doctor" {
  run             = "claude --model sonnet"
  on_idle         = "done"
  max_concurrency = 1
  prompt          = "Run diagnostics..."
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let agent = &runbook.agents["doctor"];
    assert_eq!(agent.max_concurrency, Some(1));
}

#[test]
fn parse_agent_max_concurrency_default() {
    let hcl = r#"
agent "doctor" {
  run     = "claude --model sonnet"
  on_idle = "done"
  prompt  = "Run diagnostics..."
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let agent = &runbook.agents["doctor"];
    assert_eq!(agent.max_concurrency, None);
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
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("max_concurrency must be >= 1"),
        "error should mention min value: {}",
        msg
    );
}

#[test]
fn error_cron_unknown_job() {
    let hcl = r#"
cron "janitor" {
  interval = "30m"
  run      = { job = "nonexistent" }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown job 'nonexistent'"),
        "error should mention unknown job: {}",
        msg
    );
}
