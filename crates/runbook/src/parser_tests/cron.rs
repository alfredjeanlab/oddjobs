// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::{parse_runbook, parse_runbook_with_format, Format, ParseError};

#[test]
fn parse_hcl_cron_valid() {
    let hcl = r#"
pipeline "cleanup" {
  step "run" {
    run = "echo cleanup"
  }
}

cron "janitor" {
  interval = "30m"
  run      = { pipeline = "cleanup" }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    assert!(runbook.crons.contains_key("janitor"));
    let cron = &runbook.crons["janitor"];
    assert_eq!(cron.name, "janitor");
    assert_eq!(cron.interval, "30m");
    assert_eq!(cron.run.pipeline_name(), Some("cleanup"));
}

#[test]
fn parse_toml_cron_valid() {
    let toml = r#"
[pipeline.deploy]
[[pipeline.deploy.step]]
name = "run"
run = "echo deploy"

[cron.nightly]
interval = "24h"
run = { pipeline = "deploy" }
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.crons.contains_key("nightly"));
    let cron = &runbook.crons["nightly"];
    assert_eq!(cron.name, "nightly");
    assert_eq!(cron.interval, "24h");
    assert_eq!(cron.run.pipeline_name(), Some("deploy"));
}

#[test]
fn error_cron_invalid_interval() {
    let hcl = r#"
pipeline "cleanup" {
  step "run" {
    run = "echo cleanup"
  }
}

cron "janitor" {
  interval = "invalid"
  run      = { pipeline = "cleanup" }
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
fn error_cron_non_pipeline_run() {
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
        msg.contains("cron run must reference a pipeline"),
        "error should mention pipeline requirement: {}",
        msg
    );
}

#[test]
fn error_cron_unknown_pipeline() {
    let hcl = r#"
cron "janitor" {
  interval = "30m"
  run      = { pipeline = "nonexistent" }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown pipeline 'nonexistent'"),
        "error should mention unknown pipeline: {}",
        msg
    );
}
