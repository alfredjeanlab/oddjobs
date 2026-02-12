// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::parser::{parse_runbook, parse_runbook_with_format, Format};

fn sample_job() -> JobDef {
    JobDef {
        kind: "build".to_string(),
        name: None,
        vars: vec!["name".to_string(), "prompt".to_string()],
        defaults: HashMap::new(),
        locals: HashMap::new(),
        cwd: None,
        source: None,
        container: None,
        on_done: None,
        on_fail: None,
        on_cancel: None,
        notify: Default::default(),
        steps: vec![
            StepDef {
                name: "init".to_string(),
                run: RunDirective::Shell("git worktree add".to_string()),
                on_done: None,
                on_fail: None,
                on_cancel: None,
            },
            StepDef {
                name: "plan".to_string(),
                run: RunDirective::Agent { agent: "planner".to_string(), attach: None },
                on_done: None,
                on_fail: None,
                on_cancel: None,
            },
            StepDef {
                name: "execute".to_string(),
                run: RunDirective::Agent { agent: "executor".to_string(), attach: None },
                on_done: Some(StepTransition { step: "done".to_string() }),
                on_fail: Some(StepTransition { step: "failed".to_string() }),
                on_cancel: None,
            },
            StepDef {
                name: "done".to_string(),
                run: RunDirective::Shell("echo done".to_string()),
                on_done: None,
                on_fail: None,
                on_cancel: None,
            },
            StepDef {
                name: "failed".to_string(),
                run: RunDirective::Shell("echo failed".to_string()),
                on_done: None,
                on_fail: None,
                on_cancel: None,
            },
        ],
    }
}

#[test]
fn job_step_lookup() {
    let p = sample_job();
    assert!(p.get_step("init").is_some());
    assert!(p.get_step("nonexistent").is_none());
}

#[test]
fn step_is_shell() {
    let p = sample_job();
    assert!(p.get_step("init").unwrap().is_shell());
    assert!(!p.get_step("plan").unwrap().is_shell());
}

#[test]
fn step_is_agent() {
    let p = sample_job();
    assert!(!p.get_step("init").unwrap().is_agent());
    assert!(p.get_step("plan").unwrap().is_agent());
    assert_eq!(p.get_step("plan").unwrap().agent_name(), Some("planner"));
}

const ON_DONE_FAIL_TOML: &str = r#"
[job.deploy]
vars  = ["name"]
on_done = { step = "teardown" }
on_fail = { step = "cleanup" }

[[job.deploy.step]]
name = "init"
run = "echo init"

[[job.deploy.step]]
name = "teardown"
run = "echo teardown"

[[job.deploy.step]]
name = "cleanup"
run = "echo cleanup"
"#;

const ON_DONE_FAIL_HCL: &str = r#"
job "deploy" {
    vars  = ["name"]
    on_done = { step = "teardown" }
    on_fail = { step = "cleanup" }

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

#[yare::parameterized(
    toml = { ON_DONE_FAIL_TOML, Format::Toml },
    hcl  = { ON_DONE_FAIL_HCL,  Format::Hcl },
)]
fn parse_job_on_done_on_fail(input: &str, fmt: Format) {
    let runbook = parse_runbook_with_format(input, fmt).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    assert_eq!(job.on_done.as_ref().map(|t| t.step_name()), Some("teardown"));
    assert_eq!(job.on_fail.as_ref().map(|t| t.step_name()), Some("cleanup"));
}

const STEP_TRANSITION_TOML: &str = r#"
[job.deploy]
vars = ["name"]

[[job.deploy.step]]
name = "init"
run = "echo init"

[job.deploy.step.on_done]
step = "next"

[[job.deploy.step]]
name = "next"
run = "echo next"
"#;

const STEP_TRANSITION_HCL: &str = r#"
job "deploy" {
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

#[yare::parameterized(
    toml = { STEP_TRANSITION_TOML, Format::Toml },
    hcl  = { STEP_TRANSITION_HCL,  Format::Hcl },
)]
fn parse_structured_step_transition(input: &str, fmt: Format) {
    let runbook = parse_runbook_with_format(input, fmt).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    let init = job.get_step("init").unwrap();
    assert_eq!(init.on_done.as_ref().map(|t| t.step_name()), Some("next"));
}

#[test]
fn parse_job_without_lifecycle_hooks() {
    let toml = r#"
[job.simple]
vars  = ["name"]

[[job.simple.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("simple").unwrap();
    assert!(job.on_done.is_none());
    assert!(job.on_fail.is_none());
}

const NOTIFY_TOML: &str = r#"
[job.deploy]
vars  = ["env"]

[job.deploy.notify]
on_start = "Deploy started: ${var.env}"
on_done  = "Deploy complete: ${var.env}"
on_fail  = "Deploy failed: ${var.env}"

[[job.deploy.step]]
name = "init"
run = "echo init"
"#;

const NOTIFY_HCL: &str = r#"
job "deploy" {
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

#[yare::parameterized(
    toml = { NOTIFY_TOML, Format::Toml },
    hcl  = { NOTIFY_HCL,  Format::Hcl },
)]
fn parse_job_notify(input: &str, fmt: Format) {
    let runbook = parse_runbook_with_format(input, fmt).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    assert_eq!(job.notify.on_start.as_deref(), Some("Deploy started: ${var.env}"));
    assert_eq!(job.notify.on_done.as_deref(), Some("Deploy complete: ${var.env}"));
    assert_eq!(job.notify.on_fail.as_deref(), Some("Deploy failed: ${var.env}"));
}

#[test]
fn parse_job_notify_partial() {
    let toml = r#"
[job.deploy]
vars = ["env"]

[job.deploy.notify]
on_done = "Done!"

[[job.deploy.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    assert!(job.notify.on_start.is_none());
    assert_eq!(job.notify.on_done.as_deref(), Some("Done!"));
    assert!(job.notify.on_fail.is_none());
}

#[test]
fn parse_job_notify_defaults_to_empty() {
    let toml = r#"
[job.simple]
vars = ["name"]

[[job.simple.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("simple").unwrap();
    assert!(job.notify.on_start.is_none());
    assert!(job.notify.on_done.is_none());
    assert!(job.notify.on_fail.is_none());
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
fn parse_hcl_job_locals() {
    let hcl = r#"
job "build" {
    vars = ["name"]

    locals {
        repo   = "$(git rev-parse --show-toplevel)"
        branch = "feature/${var.name}-${source.nonce}"
        title  = "feat: ${var.name}"
    }

    step "init" {
        run = "echo ${local.branch}"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("build").unwrap();
    assert_eq!(job.locals.len(), 3);
    assert_eq!(job.locals.get("repo").unwrap(), "$(git rev-parse --show-toplevel)");
    assert_eq!(job.locals.get("branch").unwrap(), "feature/${var.name}-${source.nonce}");
    assert_eq!(job.locals.get("title").unwrap(), "feat: ${var.name}");
}

#[test]
fn parse_toml_job_locals() {
    let runbook = parse_runbook(
        r#"
[job.build]
vars = ["name"]

[job.build.locals]
repo   = "$(git rev-parse --show-toplevel)"
branch = "feature/${var.name}"

[[job.build.step]]
name = "init"
run  = "echo init"
"#,
    )
    .unwrap();
    let job = runbook.get_job("build").unwrap();
    assert_eq!(job.locals.len(), 2);
    assert_eq!(job.locals.get("repo").unwrap(), "$(git rev-parse --show-toplevel)");
    assert_eq!(job.locals.get("branch").unwrap(), "feature/${var.name}");
}

#[test]
fn parse_job_locals_defaults_to_empty() {
    let toml = r#"
[job.simple]
vars = ["name"]

[[job.simple.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("simple").unwrap();
    assert!(job.locals.is_empty());
}

const ON_CANCEL_TOML: &str = r#"
[job.deploy]
vars  = ["name"]
on_cancel = { step = "cleanup" }

[[job.deploy.step]]
name = "init"
run = "echo init"
on_cancel = { step = "teardown" }

[[job.deploy.step]]
name = "teardown"
run = "echo teardown"

[[job.deploy.step]]
name = "cleanup"
run = "echo cleanup"
"#;

const ON_CANCEL_HCL: &str = r#"
job "deploy" {
    vars  = ["name"]
    on_cancel = { step = "cleanup" }

    step "init" {
        run       = "echo init"
        on_cancel = { step = "teardown" }
    }

    step "teardown" {
        run = "echo teardown"
    }

    step "cleanup" {
        run = "echo cleanup"
    }
}
"#;

#[yare::parameterized(
    toml = { ON_CANCEL_TOML, Format::Toml },
    hcl  = { ON_CANCEL_HCL,  Format::Hcl },
)]
fn parse_job_on_cancel(input: &str, fmt: Format) {
    let runbook = parse_runbook_with_format(input, fmt).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    assert_eq!(job.on_cancel.as_ref().map(|t| t.step_name()), Some("cleanup"));
    let init = job.get_step("init").unwrap();
    assert_eq!(init.on_cancel.as_ref().map(|t| t.step_name()), Some("teardown"));
}

#[test]
fn parse_job_without_on_cancel() {
    let toml = r#"
[job.simple]
vars  = ["name"]

[[job.simple.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("simple").unwrap();
    assert!(job.on_cancel.is_none());
    let init = job.get_step("init").unwrap();
    assert!(init.on_cancel.is_none());
}

const NAME_TEMPLATE_HCL: &str = r#"
job "fix" {
    name = "${var.bug.title}"
    vars = ["bug"]

    step "init" {
        run = "echo init"
    }
}
"#;

const NAME_TEMPLATE_TOML: &str = r#"
[job.deploy]
name = "${var.env}"
vars = ["env"]

[[job.deploy.step]]
name = "init"
run = "echo init"
"#;

#[yare::parameterized(
    hcl  = { NAME_TEMPLATE_HCL,  Format::Hcl,  "fix",    "${var.bug.title}" },
    toml = { NAME_TEMPLATE_TOML, Format::Toml,  "deploy", "${var.env}" },
)]
fn parse_job_name_template(input: &str, fmt: Format, kind: &str, name_tmpl: &str) {
    let runbook = parse_runbook_with_format(input, fmt).unwrap();
    let job = runbook.get_job(kind).unwrap();
    assert_eq!(job.kind, kind);
    assert_eq!(job.name.as_deref(), Some(name_tmpl));
}

#[test]
fn parse_job_without_name_template() {
    let runbook = parse_runbook_with_format(
        r#"
job "build" {
    vars = ["name"]

    step "init" {
        run = "echo init"
    }
}
"#,
        Format::Hcl,
    )
    .unwrap();
    let job = runbook.get_job("build").unwrap();
    assert_eq!(job.kind, "build");
    assert!(job.name.is_none());
}

const SOURCE_FOLDER_HCL: &str = r#"
job "test" {
    vars = ["name"]
    source = "folder"

    step "init" {
        run = "echo init"
    }
}
"#;

const SOURCE_FOLDER_TOML: &str = r#"
[job.test]
vars = ["name"]
source = "folder"

[[job.test.step]]
name = "init"
run = "echo init"
"#;

#[yare::parameterized(
    hcl  = { SOURCE_FOLDER_HCL,  Format::Hcl },
    toml = { SOURCE_FOLDER_TOML, Format::Toml },
)]
fn parse_source_folder(input: &str, fmt: Format) {
    let runbook = parse_runbook_with_format(input, fmt).unwrap();
    let job = runbook.get_job("test").unwrap();
    assert_eq!(job.source, Some(WorkspaceConfig::Simple(WorkspaceType::Folder)));
    assert!(!job.source.as_ref().unwrap().is_git_worktree());
}

#[test]
fn parse_hcl_source_git_true() {
    let runbook = parse_runbook_with_format(
        r#"
job "test" {
    vars = ["name"]

    source {
        git = true
    }

    step "init" {
        run = "echo init"
    }
}
"#,
        Format::Hcl,
    )
    .unwrap();
    let job = runbook.get_job("test").unwrap();
    assert!(job.source.as_ref().unwrap().is_git_worktree());
    assert_eq!(
        job.source,
        Some(WorkspaceConfig::Block(WorkspaceBlock {
            git: GitWorkspaceMode::Worktree,
            branch: None,
            from_ref: None,
        }))
    );
}

#[test]
fn workspace_config_is_git_worktree() {
    let folder = WorkspaceConfig::Simple(WorkspaceType::Folder);
    assert!(!folder.is_git_worktree());

    let worktree = WorkspaceConfig::Block(WorkspaceBlock {
        git: GitWorkspaceMode::Worktree,
        branch: None,
        from_ref: None,
    });
    assert!(worktree.is_git_worktree());
}

#[test]
fn parse_hcl_source_git_with_branch() {
    let hcl = r#"
job "test" {
    vars = ["name"]

    source {
        git    = true
        branch = "feat/${var.name}"
    }

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("test").unwrap();
    assert!(job.source.as_ref().unwrap().is_git_worktree());
    assert_eq!(
        job.source,
        Some(WorkspaceConfig::Block(WorkspaceBlock {
            git: GitWorkspaceMode::Worktree,
            branch: Some("feat/${var.name}".to_string()),
            from_ref: None,
        }))
    );
}

#[test]
fn parse_hcl_source_git_with_ref() {
    let hcl = r#"
job "test" {
    vars = ["name"]

    source {
        git = true
        ref = "origin/main"
    }

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("test").unwrap();
    assert!(job.source.as_ref().unwrap().is_git_worktree());
    assert_eq!(
        job.source,
        Some(WorkspaceConfig::Block(WorkspaceBlock {
            git: GitWorkspaceMode::Worktree,
            branch: None,
            from_ref: Some("origin/main".to_string()),
        }))
    );
}

#[test]
fn parse_hcl_source_git_with_branch_and_ref() {
    let hcl = r#"
job "test" {
    vars = ["name"]

    source {
        git    = true
        branch = "feat/${var.name}-${source.nonce}"
        ref    = "origin/main"
    }

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("test").unwrap();
    assert!(job.source.as_ref().unwrap().is_git_worktree());
    assert_eq!(
        job.source,
        Some(WorkspaceConfig::Block(WorkspaceBlock {
            git: GitWorkspaceMode::Worktree,
            branch: Some("feat/${var.name}-${source.nonce}".to_string()),
            from_ref: Some("origin/main".to_string()),
        }))
    );
}
