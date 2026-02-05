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
        workspace: None,
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
                run: RunDirective::Agent {
                    agent: "planner".to_string(),
                    attach: None,
                },
                on_done: None,
                on_fail: None,
                on_cancel: None,
            },
            StepDef {
                name: "execute".to_string(),
                run: RunDirective::Agent {
                    agent: "executor".to_string(),
                    attach: None,
                },
                on_done: Some(StepTransition {
                    step: "done".to_string(),
                }),
                on_fail: Some(StepTransition {
                    step: "failed".to_string(),
                }),
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

#[test]
fn parse_toml_job_on_done_on_fail() {
    let toml = r#"
[job.deploy]
vars  = ["name"]
on_done = "teardown"
on_fail = "cleanup"

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
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    assert_eq!(
        job.on_done.as_ref().map(|t| t.step_name()),
        Some("teardown")
    );
    assert_eq!(job.on_fail.as_ref().map(|t| t.step_name()), Some("cleanup"));
}

#[test]
fn parse_hcl_job_on_done_on_fail() {
    let hcl = r#"
job "deploy" {
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
    let job = runbook.get_job("deploy").unwrap();
    assert_eq!(
        job.on_done.as_ref().map(|t| t.step_name()),
        Some("teardown")
    );
    assert_eq!(job.on_fail.as_ref().map(|t| t.step_name()), Some("cleanup"));
}

#[test]
fn parse_toml_structured_step_transition() {
    let toml = r#"
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
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    let init = job.get_step("init").unwrap();
    assert_eq!(init.on_done.as_ref().map(|t| t.step_name()), Some("next"));
}

#[test]
fn parse_hcl_structured_step_transition() {
    let hcl = r#"
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
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
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

#[test]
fn parse_toml_job_notify() {
    let toml = r#"
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
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    assert_eq!(
        job.notify.on_start.as_deref(),
        Some("Deploy started: ${var.env}")
    );
    assert_eq!(
        job.notify.on_done.as_deref(),
        Some("Deploy complete: ${var.env}")
    );
    assert_eq!(
        job.notify.on_fail.as_deref(),
        Some("Deploy failed: ${var.env}")
    );
}

#[test]
fn parse_hcl_job_notify() {
    let hcl = r#"
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
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    assert_eq!(
        job.notify.on_start.as_deref(),
        Some("Deploy started: ${var.env}")
    );
    assert_eq!(
        job.notify.on_done.as_deref(),
        Some("Deploy complete: ${var.env}")
    );
    assert_eq!(
        job.notify.on_fail.as_deref(),
        Some("Deploy failed: ${var.env}")
    );
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
        branch = "feature/${var.name}-${workspace.nonce}"
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
    assert_eq!(
        job.locals.get("repo").unwrap(),
        "$(git rev-parse --show-toplevel)"
    );
    assert_eq!(
        job.locals.get("branch").unwrap(),
        "feature/${var.name}-${workspace.nonce}"
    );
    assert_eq!(job.locals.get("title").unwrap(), "feat: ${var.name}");
}

#[test]
fn parse_toml_job_locals() {
    let toml = r#"
[job.build]
vars = ["name"]

[job.build.locals]
repo   = "$(git rev-parse --show-toplevel)"
branch = "feature/${var.name}"

[[job.build.step]]
name = "init"
run  = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("build").unwrap();
    assert_eq!(job.locals.len(), 2);
    assert_eq!(
        job.locals.get("repo").unwrap(),
        "$(git rev-parse --show-toplevel)"
    );
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

#[test]
fn parse_toml_job_on_cancel() {
    let toml = r#"
[job.deploy]
vars  = ["name"]
on_cancel = "cleanup"

[[job.deploy.step]]
name = "init"
run = "echo init"
on_cancel = "teardown"

[[job.deploy.step]]
name = "teardown"
run = "echo teardown"

[[job.deploy.step]]
name = "cleanup"
run = "echo cleanup"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    assert_eq!(
        job.on_cancel.as_ref().map(|t| t.step_name()),
        Some("cleanup")
    );
    let init = job.get_step("init").unwrap();
    assert_eq!(
        init.on_cancel.as_ref().map(|t| t.step_name()),
        Some("teardown")
    );
}

#[test]
fn parse_hcl_job_on_cancel() {
    let hcl = r#"
job "deploy" {
    vars  = ["name"]
    on_cancel = "cleanup"

    step "init" {
        run       = "echo init"
        on_cancel = "teardown"
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
    let job = runbook.get_job("deploy").unwrap();
    assert_eq!(
        job.on_cancel.as_ref().map(|t| t.step_name()),
        Some("cleanup")
    );
    let init = job.get_step("init").unwrap();
    assert_eq!(
        init.on_cancel.as_ref().map(|t| t.step_name()),
        Some("teardown")
    );
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

#[test]
fn parse_hcl_job_name_template() {
    let hcl = r#"
job "fix" {
    name = "${var.bug.title}"
    vars = ["bug"]

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("fix").unwrap();
    assert_eq!(job.kind, "fix");
    assert_eq!(job.name.as_deref(), Some("${var.bug.title}"));
}

#[test]
fn parse_job_without_name_template() {
    let hcl = r#"
job "build" {
    vars = ["name"]

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("build").unwrap();
    assert_eq!(job.kind, "build");
    assert!(job.name.is_none());
}

#[test]
fn parse_toml_job_name_template() {
    let toml = r#"
[job.deploy]
name = "${var.env}"
vars = ["env"]

[[job.deploy.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("deploy").unwrap();
    assert_eq!(job.kind, "deploy");
    assert_eq!(job.name.as_deref(), Some("${var.env}"));
}

#[test]
fn parse_hcl_workspace_folder() {
    let hcl = r#"
job "test" {
    vars = ["name"]
    workspace = "folder"

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("test").unwrap();
    assert_eq!(
        job.workspace,
        Some(WorkspaceConfig::Simple(WorkspaceType::Folder))
    );
    assert!(!job.workspace.as_ref().unwrap().is_git_worktree());
}

#[test]
fn parse_hcl_workspace_git_worktree() {
    let hcl = r#"
job "test" {
    vars = ["name"]

    workspace {
        git = "worktree"
    }

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("test").unwrap();
    assert!(job.workspace.as_ref().unwrap().is_git_worktree());
    assert_eq!(
        job.workspace,
        Some(WorkspaceConfig::Block(WorkspaceBlock {
            git: GitWorkspaceMode::Worktree,
            branch: None,
            from_ref: None,
        }))
    );
}

#[test]
fn parse_toml_workspace_folder() {
    let toml = r#"
[job.test]
vars = ["name"]
workspace = "folder"

[[job.test.step]]
name = "init"
run = "echo init"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let job = runbook.get_job("test").unwrap();
    assert_eq!(
        job.workspace,
        Some(WorkspaceConfig::Simple(WorkspaceType::Folder))
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
fn parse_hcl_workspace_git_worktree_with_branch() {
    let hcl = r#"
job "test" {
    vars = ["name"]

    workspace {
        git    = "worktree"
        branch = "feat/${var.name}"
    }

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("test").unwrap();
    assert!(job.workspace.as_ref().unwrap().is_git_worktree());
    assert_eq!(
        job.workspace,
        Some(WorkspaceConfig::Block(WorkspaceBlock {
            git: GitWorkspaceMode::Worktree,
            branch: Some("feat/${var.name}".to_string()),
            from_ref: None,
        }))
    );
}

#[test]
fn parse_hcl_workspace_git_worktree_with_ref() {
    let hcl = r#"
job "test" {
    vars = ["name"]

    workspace {
        git = "worktree"
        ref = "origin/main"
    }

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("test").unwrap();
    assert!(job.workspace.as_ref().unwrap().is_git_worktree());
    assert_eq!(
        job.workspace,
        Some(WorkspaceConfig::Block(WorkspaceBlock {
            git: GitWorkspaceMode::Worktree,
            branch: None,
            from_ref: Some("origin/main".to_string()),
        }))
    );
}

#[test]
fn parse_hcl_workspace_git_worktree_with_branch_and_ref() {
    let hcl = r#"
job "test" {
    vars = ["name"]

    workspace {
        git    = "worktree"
        branch = "feat/${var.name}-${workspace.nonce}"
        ref    = "origin/main"
    }

    step "init" {
        run = "echo init"
    }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = runbook.get_job("test").unwrap();
    assert!(job.workspace.as_ref().unwrap().is_git_worktree());
    assert_eq!(
        job.workspace,
        Some(WorkspaceConfig::Block(WorkspaceBlock {
            git: GitWorkspaceMode::Worktree,
            branch: Some("feat/${var.name}-${workspace.nonce}".to_string()),
            from_ref: Some("origin/main".to_string()),
        }))
    );
}
