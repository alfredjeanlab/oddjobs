// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::parser::{parse_runbook, parse_runbook_with_format, Format};

fn sample_pipeline() -> PipelineDef {
    PipelineDef {
        name: "build".to_string(),
        vars: vec!["name".to_string(), "prompt".to_string()],
        defaults: HashMap::new(),
        locals: HashMap::new(),
        cwd: None,
        workspace: None,
        on_done: None,
        on_fail: None,
        notify: Default::default(),
        steps: vec![
            StepDef {
                name: "init".to_string(),
                run: RunDirective::Shell("git worktree add".to_string()),
                on_done: None,
                on_fail: None,
            },
            StepDef {
                name: "plan".to_string(),
                run: RunDirective::Agent {
                    agent: "planner".to_string(),
                },
                on_done: None,
                on_fail: None,
            },
            StepDef {
                name: "execute".to_string(),
                run: RunDirective::Agent {
                    agent: "executor".to_string(),
                },
                on_done: Some(StepTransition {
                    step: "done".to_string(),
                }),
                on_fail: Some(StepTransition {
                    step: "failed".to_string(),
                }),
            },
            StepDef {
                name: "done".to_string(),
                run: RunDirective::Shell("echo done".to_string()),
                on_done: None,
                on_fail: None,
            },
            StepDef {
                name: "failed".to_string(),
                run: RunDirective::Shell("echo failed".to_string()),
                on_done: None,
                on_fail: None,
            },
        ],
    }
}

#[test]
fn pipeline_step_lookup() {
    let p = sample_pipeline();
    assert!(p.get_step("init").is_some());
    assert!(p.get_step("nonexistent").is_none());
}

#[test]
fn step_is_shell() {
    let p = sample_pipeline();
    assert!(p.get_step("init").unwrap().is_shell());
    assert!(!p.get_step("plan").unwrap().is_shell());
}

#[test]
fn step_is_agent() {
    let p = sample_pipeline();
    assert!(!p.get_step("init").unwrap().is_agent());
    assert!(p.get_step("plan").unwrap().is_agent());
    assert_eq!(p.get_step("plan").unwrap().agent_name(), Some("planner"));
}

#[test]
fn parse_toml_pipeline_on_done_on_fail() {
    let toml = r#"
[pipeline.deploy]
vars  = ["name"]
on_done = "teardown"
on_fail = "cleanup"

[[pipeline.deploy.step]]
name = "init"
run = "echo init"

[[pipeline.deploy.step]]
name = "teardown"
run = "echo teardown"

[[pipeline.deploy.step]]
name = "cleanup"
run = "echo cleanup"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let pipeline = runbook.get_pipeline("deploy").unwrap();
    assert_eq!(
        pipeline.on_done.as_ref().map(|t| t.step_name()),
        Some("teardown")
    );
    assert_eq!(
        pipeline.on_fail.as_ref().map(|t| t.step_name()),
        Some("cleanup")
    );
}

#[test]
fn parse_hcl_pipeline_on_done_on_fail() {
    let hcl = r#"
pipeline "deploy" {
    vars  = ["name"]
    on_done = "teardown"
    on_fail = "cleanup"

    step "init" {
        run = "echo init"
    }

    step "teardown" {
        run = "echo teardown"
    }

    step "cleanup" {
        run = "echo cleanup"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let pipeline = runbook.get_pipeline("deploy").unwrap();
    assert_eq!(
        pipeline.on_done.as_ref().map(|t| t.step_name()),
        Some("teardown")
    );
    assert_eq!(
        pipeline.on_fail.as_ref().map(|t| t.step_name()),
        Some("cleanup")
    );
}

#[test]
fn parse_toml_structured_step_transition() {
    let toml = r#"
[pipeline.deploy]
vars = ["name"]

[[pipeline.deploy.step]]
name = "init"
run = "echo init"

[pipeline.deploy.step.on_done]
step = "next"

[[pipeline.deploy.step]]
name = "next"
run = "echo next"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let pipeline = runbook.get_pipeline("deploy").unwrap();
    let init = pipeline.get_step("init").unwrap();
    assert_eq!(init.on_done.as_ref().map(|t| t.step_name()), Some("next"));
}

#[test]
fn parse_hcl_structured_step_transition() {
    let hcl = r#"
pipeline "deploy" {
    vars = ["name"]

    step "init" {
        run     = "echo init"
        on_done = { step = "next" }
    }

    step "next" {
        run = "echo next"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let pipeline = runbook.get_pipeline("deploy").unwrap();
    let init = pipeline.get_step("init").unwrap();
    assert_eq!(init.on_done.as_ref().map(|t| t.step_name()), Some("next"));
}

#[test]
fn parse_pipeline_without_lifecycle_hooks() {
    let toml = r#"
[pipeline.simple]
vars  = ["name"]

[[pipeline.simple.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let pipeline = runbook.get_pipeline("simple").unwrap();
    assert!(pipeline.on_done.is_none());
    assert!(pipeline.on_fail.is_none());
}

#[test]
fn parse_toml_pipeline_notify() {
    let toml = r#"
[pipeline.deploy]
vars  = ["env"]

[pipeline.deploy.notify]
on_start = "Deploy started: ${var.env}"
on_done  = "Deploy complete: ${var.env}"
on_fail  = "Deploy failed: ${var.env}"

[[pipeline.deploy.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let pipeline = runbook.get_pipeline("deploy").unwrap();
    assert_eq!(
        pipeline.notify.on_start.as_deref(),
        Some("Deploy started: ${var.env}")
    );
    assert_eq!(
        pipeline.notify.on_done.as_deref(),
        Some("Deploy complete: ${var.env}")
    );
    assert_eq!(
        pipeline.notify.on_fail.as_deref(),
        Some("Deploy failed: ${var.env}")
    );
}

#[test]
fn parse_hcl_pipeline_notify() {
    let hcl = r#"
pipeline "deploy" {
    vars = ["env"]

    notify {
        on_start = "Deploy started: ${var.env}"
        on_done  = "Deploy complete: ${var.env}"
        on_fail  = "Deploy failed: ${var.env}"
    }

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let pipeline = runbook.get_pipeline("deploy").unwrap();
    assert_eq!(
        pipeline.notify.on_start.as_deref(),
        Some("Deploy started: ${var.env}")
    );
    assert_eq!(
        pipeline.notify.on_done.as_deref(),
        Some("Deploy complete: ${var.env}")
    );
    assert_eq!(
        pipeline.notify.on_fail.as_deref(),
        Some("Deploy failed: ${var.env}")
    );
}

#[test]
fn parse_pipeline_notify_partial() {
    let toml = r#"
[pipeline.deploy]
vars = ["env"]

[pipeline.deploy.notify]
on_done = "Done!"

[[pipeline.deploy.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let pipeline = runbook.get_pipeline("deploy").unwrap();
    assert!(pipeline.notify.on_start.is_none());
    assert_eq!(pipeline.notify.on_done.as_deref(), Some("Done!"));
    assert!(pipeline.notify.on_fail.is_none());
}

#[test]
fn parse_pipeline_notify_defaults_to_empty() {
    let toml = r#"
[pipeline.simple]
vars = ["name"]

[[pipeline.simple.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let pipeline = runbook.get_pipeline("simple").unwrap();
    assert!(pipeline.notify.on_start.is_none());
    assert!(pipeline.notify.on_done.is_none());
    assert!(pipeline.notify.on_fail.is_none());
}

#[test]
fn notify_config_render_interpolates() {
    let vars: HashMap<String, String> = [
        ("var.env".to_string(), "production".to_string()),
        ("name".to_string(), "my-deploy".to_string()),
    ]
    .into_iter()
    .collect();
    let result = NotifyConfig::render("Deploy ${var.env} for ${name}", &vars);
    assert_eq!(result, "Deploy production for my-deploy");
}

#[test]
fn parse_hcl_pipeline_locals() {
    let hcl = r#"
pipeline "build" {
    vars = ["name"]

    locals {
        repo   = "$(git rev-parse --show-toplevel)"
        branch = "feature/${var.name}-${workspace.nonce}"
        title  = "feat: ${var.name}"
    }

    step "init" {
        run = "echo ${local.branch}"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let pipeline = runbook.get_pipeline("build").unwrap();
    assert_eq!(pipeline.locals.len(), 3);
    assert_eq!(
        pipeline.locals.get("repo").unwrap(),
        "$(git rev-parse --show-toplevel)"
    );
    assert_eq!(
        pipeline.locals.get("branch").unwrap(),
        "feature/${var.name}-${workspace.nonce}"
    );
    assert_eq!(pipeline.locals.get("title").unwrap(), "feat: ${var.name}");
}

#[test]
fn parse_toml_pipeline_locals() {
    let toml = r#"
[pipeline.build]
vars = ["name"]

[pipeline.build.locals]
repo   = "$(git rev-parse --show-toplevel)"
branch = "feature/${var.name}"

[[pipeline.build.step]]
name = "init"
run  = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let pipeline = runbook.get_pipeline("build").unwrap();
    assert_eq!(pipeline.locals.len(), 2);
    assert_eq!(
        pipeline.locals.get("repo").unwrap(),
        "$(git rev-parse --show-toplevel)"
    );
    assert_eq!(
        pipeline.locals.get("branch").unwrap(),
        "feature/${var.name}"
    );
}

#[test]
fn parse_pipeline_locals_defaults_to_empty() {
    let toml = r#"
[pipeline.simple]
vars = ["name"]

[[pipeline.simple.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let pipeline = runbook.get_pipeline("simple").unwrap();
    assert!(pipeline.locals.is_empty());
}
