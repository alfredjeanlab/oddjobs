// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Locals interpolation and workspace variable tests

use super::*;

/// Runbook with locals that reference job vars via ${var.*}
const RUNBOOK_WITH_LOCALS: &str = r#"
[command.build]
args = "<name> <instructions>"
run = { job = "build" }

[job.build]
input = ["name", "instructions"]

[job.build.locals]
branch = "feature/${var.name}"
title = "feat(${var.name}): ${var.instructions}"

[[job.build.step]]
name = "init"
run = "echo ${local.branch} ${local.title}"
"#;

#[tokio::test]
async fn locals_interpolate_var_references() {
    let ctx = setup_with_runbook(RUNBOOK_WITH_LOCALS).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [
                ("name".to_string(), "auth".to_string()),
                ("instructions".to_string(), "add login".to_string()),
            ]
            .into_iter()
            .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let job = ctx.runtime.get_job(&job_id).unwrap();

    assert_eq!(
        job.vars.get("local.branch").map(String::as_str),
        Some("feature/auth"),
        "local.branch should interpolate ${{var.name}}"
    );
    assert_eq!(
        job.vars.get("local.title").map(String::as_str),
        Some("feat(auth): add login"),
        "local.title should interpolate ${{var.name}} and ${{var.instructions}}"
    );
}

/// Runbook with locals that reference workspace variables
const RUNBOOK_LOCALS_WITH_WORKSPACE: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]
workspace = "folder"

[job.build.locals]
branch = "feature/${var.name}-${workspace.nonce}"

[[job.build.step]]
name = "init"
run = "echo ${local.branch}"
"#;

#[tokio::test]
async fn locals_interpolate_workspace_variables() {
    let ctx = setup_with_runbook(RUNBOOK_LOCALS_WITH_WORKSPACE).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "auth".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let job = ctx.runtime.get_job(&job_id).unwrap();

    let branch = job.vars.get("local.branch").cloned().unwrap_or_default();
    assert!(
        branch.starts_with("feature/auth-"),
        "local.branch should start with 'feature/auth-', got: {branch}"
    );
    // Should NOT contain unresolved template variables
    assert!(
        !branch.contains("${"),
        "local.branch should not contain unresolved variables, got: {branch}"
    );
}

/// Locals containing shell expressions $(...) are eagerly evaluated at job
/// creation time. The output of the shell command is stored as plain data.
const RUNBOOK_LOCALS_SHELL_SUBST: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]

[job.build.locals]
repo = "$(echo /some/repo)"

[[job.build.step]]
name = "init"
run = "echo ${local.repo}"
"#;

#[tokio::test]
async fn locals_eagerly_evaluate_shell_expressions() {
    let ctx = setup_with_runbook(RUNBOOK_LOCALS_SHELL_SUBST).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let job = ctx.runtime.get_job(&job_id).unwrap();

    // After eager evaluation, $(echo /some/repo) should be resolved
    assert_eq!(
        job.vars.get("local.repo").map(String::as_str),
        Some("/some/repo"),
        "Shell command substitution should be eagerly evaluated in locals"
    );
}

/// Runbook with folder workspace and variables referencing workspace.nonce
/// Tests runtime interpolation of workspace variables (not just parsing).
const RUNBOOK_WORKSPACE_VAR_INTERPOLATION: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]
workspace = "folder"

[job.build.locals]
branch = "feature/${var.name}-${workspace.nonce}"

[[job.build.step]]
name = "init"
run = "echo ${local.branch}"
"#;

#[tokio::test]
async fn workspace_variables_interpolate_at_runtime() {
    let ctx = setup_with_runbook(RUNBOOK_WORKSPACE_VAR_INTERPOLATION).await;

    handle_event_chain(
        &ctx,
        command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "auth".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ),
    )
    .await;

    let job_id = ctx.runtime.jobs().keys().next().unwrap().clone();
    let job = ctx.runtime.get_job(&job_id).unwrap();

    // workspace.nonce should be set
    let nonce = job.vars.get("workspace.nonce").cloned().unwrap_or_default();
    assert!(!nonce.is_empty(), "workspace.nonce should be set");

    // local.branch should be interpolated with var.name and workspace.nonce
    let branch = job.vars.get("local.branch").cloned().unwrap_or_default();
    assert!(
        branch.starts_with("feature/auth-"),
        "local.branch should start with 'feature/auth-', got: {branch}"
    );
    assert!(
        !branch.contains("${"),
        "local.branch should not contain unresolved variables, got: {branch}"
    );

    // Verify the nonce portion is in the branch
    assert!(
        branch.ends_with(&nonce),
        "local.branch should end with workspace.nonce '{nonce}', got: {branch}"
    );
}
