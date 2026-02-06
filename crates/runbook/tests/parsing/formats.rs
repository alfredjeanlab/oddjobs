// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Multi-format parsing tests: TOML, JSON, and HCL.

use oj_runbook::{parse_runbook, parse_runbook_with_format, Format, Runbook};

fn assert_sample_build_runbook(runbook: &Runbook) {
    let cmd = &runbook.commands["build"];
    assert!(cmd.run.is_job());
    assert_eq!(cmd.run.job_name(), Some("build"));
    assert_eq!(cmd.args.positional.len(), 2);
    assert_eq!(cmd.args.positional[0].name, "name");
    assert_eq!(cmd.args.positional[1].name, "prompt");
    assert_eq!(cmd.defaults.get("branch"), Some(&"main".to_string()));

    let job = &runbook.jobs["build"];
    assert_eq!(job.vars, vec!["name", "prompt"]);
    assert_eq!(job.steps.len(), 5);
    assert_eq!(job.steps[0].name, "init");
    assert!(job.steps[0].run.is_shell());
    assert_eq!(job.steps[1].name, "plan");
    assert!(job.steps[1].run.is_agent());
    assert_eq!(job.steps[1].agent_name(), Some("planner"));

    let agent = &runbook.agents["planner"];
    assert!(agent.run.contains("claude"));
    assert!(agent.env.contains_key("OJ_STEP"));
}

// ============================================================================
// TOML
// ============================================================================

#[test]
fn toml_sample_runbook() {
    let runbook = parse_runbook(include_str!("../fixtures/sample_build.toml")).unwrap();
    assert_sample_build_runbook(&runbook);
}

#[test]
fn toml_empty() {
    let runbook = parse_runbook("").unwrap();
    assert!(runbook.commands.is_empty());
    assert!(runbook.jobs.is_empty());
}

#[test]
fn toml_command_with_args_string() {
    let toml = r#"
[command.deploy]
args = "<env> [-t/--tag <version>] [-f/--force] [targets...]"
run = "deploy.sh"
[command.deploy.defaults]
tag = "latest"
"#;
    let cmd = &parse_runbook(toml).unwrap().commands["deploy"];
    assert_eq!(cmd.args.positional[0].name, "env");
    assert_eq!(cmd.args.options[0].name, "tag");
    assert_eq!(cmd.args.flags[0].name, "force");
    assert_eq!(cmd.args.variadic.as_ref().unwrap().name, "targets");
    assert_eq!(cmd.run.shell_command(), Some("deploy.sh"));
}

// ============================================================================
// JSON
// ============================================================================

#[test]
fn json_sample_runbook() {
    let runbook = super::parse_json(include_str!("../fixtures/sample_build.json"));
    assert_sample_build_runbook(&runbook);

    let job = &runbook.jobs["build"];
    assert_eq!(
        job.steps[2].on_done.as_ref().map(|t| t.step_name()),
        Some("done")
    );
    assert_eq!(
        job.steps[2].on_fail.as_ref().map(|t| t.step_name()),
        Some("failed")
    );
}

#[test]
fn json_empty() {
    let runbook = super::parse_json("{}");
    assert!(runbook.commands.is_empty());
    assert!(runbook.jobs.is_empty());
}

// ============================================================================
// HCL
// ============================================================================

#[test]
fn hcl_sample_runbook() {
    let runbook = super::parse_hcl(include_str!("../fixtures/sample_build.hcl"));
    assert_sample_build_runbook(&runbook);

    let job = &runbook.jobs["build"];
    assert_eq!(
        job.steps[2].on_done.as_ref().map(|t| t.step_name()),
        Some("done")
    );
    assert_eq!(
        job.steps[2].on_fail.as_ref().map(|t| t.step_name()),
        Some("failed")
    );
}

#[test]
fn hcl_empty() {
    let runbook = parse_runbook_with_format("", Format::Hcl).unwrap();
    assert!(runbook.commands.is_empty());
    assert!(runbook.jobs.is_empty());
}

#[test]
fn hcl_step_names_from_block_labels() {
    let hcl = r#"
job "deploy" {
  vars  = ["env"]

  step "build" {
    run     = "make build"
    on_done = "test"
  }

  step "test" {
    run     = "make test"
    on_done = "deploy"
  }

  step "deploy" {
    run = "make deploy"
  }
}
"#;
    let job = &super::parse_hcl(hcl).jobs["deploy"];
    assert_eq!(job.steps.len(), 3);
    assert_eq!(job.steps[0].name, "build");
    assert_eq!(job.steps[1].name, "test");
    assert_eq!(
        job.steps[1].on_done.as_ref().map(|t| t.step_name()),
        Some("deploy")
    );
    assert_eq!(job.steps[2].name, "deploy");
}
