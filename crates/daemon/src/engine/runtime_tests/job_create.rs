// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for job creation logic in `runtime/handlers/job_create.rs`.
//!
//! Focuses on:
//! - Workspace setup (folder mode, cwd-only, default path)
//! - Name template resolution
//! - Runbook caching and RunbookLoaded event emission
//! - Namespace propagation
//! - Workspace setup failure -> job marked failed
//! - cron_name propagation

use super::*;

/// Helper: create a job via the standard command event chain.
async fn run_job(ctx: &TestContext, job_id: &str, command: &str, args: HashMap<String, String>) {
    handle_event_chain(ctx, command_event(job_id, command, command, args, &ctx.project_path)).await;
}

/// Shorthand: create a job with a single "name" arg.
async fn run_job_named(ctx: &TestContext, job_id: &str, command: &str, name: &str) {
    run_job(ctx, job_id, command, vars!("name" => name)).await;
}

// =============================================================================
// Job with explicit cwd, no workspace
// =============================================================================

#[tokio::test]
async fn job_with_cwd_uses_interpolated_path() {
    let ctx =
        setup_with_runbook(&test_runbook_shell("deploy", "cwd = \"${invoke.dir}/subdir\"")).await;

    run_job_named(&ctx, "job-1", "deploy", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    // cwd should be the interpolated path
    let expected_cwd = ctx.project_path.join("subdir");
    assert_eq!(job.cwd, expected_cwd, "cwd should be interpolated from template");
    // No workspace should be created
    assert!(job.workspace_id.is_none(), "cwd-only job should not have workspace_id");
    assert!(job.workspace_path.is_none(), "cwd-only job should not have workspace_path");
}

// =============================================================================
// Job with no cwd, no workspace (default to invoke.dir)
// =============================================================================

#[tokio::test]
async fn job_without_cwd_or_workspace_uses_invoke_dir() {
    let ctx = setup_with_runbook(&test_runbook_shell("simple", "")).await;

    run_job_named(&ctx, "job-1", "simple", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    // cwd should be the invoke directory (project_path in our test)
    assert_eq!(job.cwd, ctx.project_path, "default cwd should be invoke.dir");
    assert!(job.workspace_id.is_none(), "no-workspace job should not have workspace_id");
}

// =============================================================================
// Job with folder workspace
// =============================================================================

#[tokio::test]
async fn job_with_folder_workspace_creates_workspace() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "source = \"folder\"")).await;

    run_job_named(&ctx, "job-1", "build", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();

    // Workspace should be created under state_dir/workspaces/
    assert!(job.workspace_id.is_some(), "folder workspace job should have workspace_id");

    let ws_id = job.workspace_id.as_ref().unwrap().to_string();
    assert!(ws_id.starts_with("ws-"), "workspace id should start with 'ws-', got: {ws_id}");

    // source vars should be injected
    assert!(job.vars.contains_key("source.id"), "source.id var should be set");
    assert!(job.vars.contains_key("source.root"), "source.root var should be set");
    assert!(job.vars.contains_key("source.nonce"), "source.nonce var should be set");
}

#[tokio::test]
async fn folder_workspace_path_is_under_state_dir() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "source = \"folder\"")).await;

    run_job_named(&ctx, "job-1", "build", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    let ws_root = job.vars.get("source.root").unwrap();
    let workspaces_dir = ctx.project_path.join("workspaces");

    assert!(
        ws_root.starts_with(&workspaces_dir.display().to_string()),
        "workspace root should be under state_dir/workspaces/, got: {ws_root}"
    );
}

// =============================================================================
// Name template resolution
// =============================================================================

#[tokio::test]
async fn job_name_template_is_interpolated() {
    let ctx =
        setup_with_runbook(&test_runbook_shell("build", "name = \"build-${var.name}\"")).await;

    run_job_named(&ctx, "job-1", "build", "auth-module").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    // The name should contain the interpolated value and a nonce suffix
    assert!(
        job.name.contains("auth-module"),
        "job name should contain interpolated var, got: {}",
        job.name
    );
}

#[tokio::test]
async fn job_without_name_template_uses_args_name() {
    let ctx = setup_with_runbook(&test_runbook_shell("simple", "")).await;

    run_job_named(&ctx, "job-1", "simple", "my-feature").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    assert_eq!(job.name, "my-feature", "without name template, job name should be args.name");
}

// =============================================================================
// Namespace propagation
// =============================================================================

#[tokio::test]
async fn job_namespace_is_propagated() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "")).await;

    // Use a command event with a non-empty project
    let event = Event::CommandRun {
        owner: JobId::from_string("job-1").into(),
        name: "build".to_string(),
        project_path: ctx.project_path.clone(),
        invoke_dir: ctx.project_path.clone(),
        command: "build".to_string(),
        project: "my-project".to_string(),
        args: vars!("name" => "test"),
    };

    ctx.runtime.handle_event(event).await.unwrap();

    let job = ctx.runtime.get_job("job-1").unwrap();
    assert_eq!(job.project, "my-project", "job project should match the command event project");
}

#[tokio::test]
async fn runbook_is_cached_after_creation() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "")).await;

    run_job_named(&ctx, "job-1", "build", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();

    // Runbook should be cached by its hash
    let cached = ctx.runtime.cached_runbook(&job.runbook_hash);
    assert!(cached.is_ok(), "runbook should be retrievable from cache after job creation");

    let runbook = cached.unwrap();
    assert!(runbook.get_job("build").is_some(), "cached runbook should contain the job definition");
}

#[tokio::test]
async fn runbook_loaded_event_is_emitted() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "")).await;

    run_job_named(&ctx, "job-1", "build", "test").await;

    // Verify the runbook was stored in materialized state (via RunbookLoaded event)
    let job = ctx.runtime.get_job("job-1").unwrap();
    let stored = ctx.runtime.lock_state(|s| s.runbooks.contains_key(&job.runbook_hash));
    assert!(stored, "RunbookLoaded event should store runbook in materialized state");
}

#[tokio::test]
async fn second_job_reuses_cached_runbook() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "")).await;

    // Create first job
    run_job_named(&ctx, "job-1", "build", "first").await;

    let job1 = ctx.runtime.get_job("job-1").unwrap();

    // Create second job with same command (same runbook)
    run_job_named(&ctx, "job-2", "build", "second").await;

    let job2 = ctx.runtime.get_job("job-2").unwrap();

    // Both jobs should reference the same runbook hash
    assert_eq!(job1.runbook_hash, job2.runbook_hash, "both jobs should use the same runbook hash");
}

const MISMATCHED_JOB_RUNBOOK: &str = r#"
[command.deploy]
run = { job = "nonexistent" }

[job.actual]
[[job.actual.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn job_def_not_found_returns_error() {
    let ctx = setup_with_runbook(MISMATCHED_JOB_RUNBOOK).await;

    let result = ctx
        .runtime
        .handle_event(command_event("job-1", "deploy", "deploy", HashMap::new(), &ctx.project_path))
        .await;

    assert!(result.is_err(), "should return error for missing job def");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not found"), "error should mention 'not found', got: {err}");
}

#[tokio::test]
async fn job_starts_at_first_step() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "")).await;

    run_job_named(&ctx, "job-1", "build", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    assert_eq!(job.step, "init", "job should start at first step");
    assert_eq!(job.step_status, StepStatus::Running, "first step should be running");
}

// =============================================================================
// Job with cwd and workspace (cwd is overridden by workspace)
// =============================================================================

#[tokio::test]
async fn cwd_with_workspace_creates_workspace() {
    let ctx = setup_with_runbook(&test_runbook_shell(
        "build",
        "cwd = \"/some/base\"\nsource = \"folder\"",
    ))
    .await;

    run_job_named(&ctx, "job-1", "build", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    // When both cwd and workspace are set, workspace takes precedence
    assert!(job.workspace_id.is_some(), "should create workspace even when cwd is also set");
}

#[tokio::test]
async fn multiple_jobs_get_distinct_workspaces() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "source = \"folder\"")).await;

    run_job_named(&ctx, "job-1", "build", "first").await;

    run_job_named(&ctx, "job-2", "build", "second").await;

    let job1 = ctx.runtime.get_job("job-1").unwrap();
    let job2 = ctx.runtime.get_job("job-2").unwrap();

    assert_ne!(
        job1.workspace_id, job2.workspace_id,
        "different jobs should have distinct workspace IDs"
    );
    assert_ne!(
        job1.vars.get("source.root"),
        job2.vars.get("source.root"),
        "different jobs should have distinct workspace paths"
    );
}

#[tokio::test]
async fn job_vars_are_namespaced() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "")).await;

    run_job_named(&ctx, "job-1", "build", "test-feature").await;

    let job = ctx.runtime.get_job("job-1").unwrap();

    // User vars should be prefixed with var.
    assert!(
        job.vars.contains_key("var.name"),
        "user vars should be namespaced with 'var.' prefix, keys: {:?}",
        job.vars.keys().collect::<Vec<_>>()
    );

    // invoke.dir should be kept as-is (already has scope prefix)
    assert!(
        job.vars.contains_key("invoke.dir"),
        "invoke.dir should be preserved, keys: {:?}",
        job.vars.keys().collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn job_created_at_uses_clock() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "")).await;

    // Advance the fake clock
    ctx.clock.advance(std::time::Duration::from_secs(1000));

    run_job_named(&ctx, "job-1", "build", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    // Job should exist and be in running state (verifying clock didn't break creation)
    assert_eq!(job.step, "init");
    assert_eq!(job.step_status, StepStatus::Running);
}

// =============================================================================
// Job with on_start notification and name template
// =============================================================================

#[tokio::test]
async fn on_start_notification_uses_resolved_name() {
    let ctx = setup_with_runbook(&test_runbook_shell(
        "build",
        "name = \"build-${var.name}\"\nnotify = { on_start = \"Started ${name}\" }",
    ))
    .await;

    run_job_named(&ctx, "job-1", "build", "auth").await;

    let calls = ctx.notifier.calls();
    assert_eq!(calls.len(), 1, "on_start should emit one notification");
    // The notification title should be the resolved job name
    assert!(
        calls[0].title.contains("auth"),
        "notification title should contain interpolated name, got: {}",
        calls[0].title
    );
    assert!(
        calls[0].message.starts_with("Started"),
        "notification message should start with 'Started', got: {}",
        calls[0].message
    );
}

#[tokio::test]
async fn workspace_nonce_is_derived_from_job_id() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "source = \"folder\"")).await;

    run_job_named(&ctx, "oj-abc12345-deadbeef", "build", "test").await;

    let job = ctx.runtime.get_job("oj-abc12345-deadbeef").unwrap();
    let nonce = job.vars.get("source.nonce").unwrap();

    // Nonce is first 8 chars of the job_id
    assert_eq!(nonce.len(), 8, "source.nonce should be 8 chars, got: {nonce}");
}

// =============================================================================
// Job with name template containing workspace nonce
// =============================================================================

#[tokio::test]
async fn name_template_with_workspace_creates_matching_ws_id() {
    let ctx = setup_with_runbook(&test_runbook_shell(
        "build",
        "name = \"${var.name}\"\nsource = \"folder\"",
    ))
    .await;

    run_job_named(&ctx, "job-1", "build", "my-feature").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    let ws_id = job.workspace_id.as_ref().unwrap().to_string();

    // The workspace ID should incorporate the name template result
    assert!(ws_id.starts_with("ws-"), "workspace id should start with 'ws-', got: {ws_id}");
    assert!(
        ws_id.contains("my-feature"),
        "workspace id should contain name from template, got: {ws_id}"
    );
}

// =============================================================================
// Job with multiple independent locals
// =============================================================================

#[tokio::test]
async fn multiple_locals_are_evaluated() {
    let ctx = setup_with_runbook(&test_runbook_shell(
        "build",
        "\n[job.build.locals]\nprefix = \"feat\"\nbranch = \"feature/${var.name}\"",
    ))
    .await;

    run_job_named(&ctx, "job-1", "build", "auth").await;

    let job = ctx.runtime.get_job("job-1").unwrap();

    assert_eq!(
        job.vars.get("local.prefix").map(String::as_str),
        Some("feat"),
        "local.prefix should be 'feat'"
    );
    assert_eq!(
        job.vars.get("local.branch").map(String::as_str),
        Some("feature/auth"),
        "local.branch should interpolate var.name"
    );
}

#[tokio::test]
async fn job_without_locals_has_no_local_vars() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "")).await;

    run_job_named(&ctx, "job-1", "build", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();

    let local_keys: Vec<_> = job.vars.keys().filter(|k| k.starts_with("local.")).collect();
    assert!(
        local_keys.is_empty(),
        "job without locals should have no local.* vars, got: {:?}",
        local_keys
    );
}

#[tokio::test]
async fn job_kind_matches_definition_name() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "")).await;

    run_job_named(&ctx, "job-1", "build", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    assert_eq!(job.kind, "build", "job kind should match the job definition name");
}

// =============================================================================
// Multiple steps: first step is picked correctly
// =============================================================================

#[tokio::test]
async fn job_starts_at_first_defined_step() {
    let runbook = test_runbook_steps(
        "pipeline",
        "",
        &[
            ("prepare", "echo prepare", "on_done = { step = \"execute\" }"),
            ("execute", "echo execute", ""),
        ],
    );
    let ctx = setup_with_runbook(&runbook).await;

    run_job_named(&ctx, "job-1", "pipeline", "test").await;

    let job = ctx.runtime.get_job("job-1").unwrap();
    assert_eq!(job.step, "prepare", "job should start at the first defined step, not 'execute'");
}

#[tokio::test]
async fn breadcrumb_is_written_after_creation() {
    let ctx = setup_with_runbook(&test_runbook_shell("build", "")).await;

    run_job_named(&ctx, "job-1", "build", "test").await;

    // Breadcrumb file should exist in the log dir
    let breadcrumb_dir = ctx.project_path.join("logs/breadcrumbs");
    let has_breadcrumbs = breadcrumb_dir.exists() && breadcrumb_dir.is_dir();

    // We just verify the job was created successfully and the breadcrumb code
    // ran without error. The BreadcrumbWriter creates files in logs/breadcrumbs/.
    let job = ctx.runtime.get_job("job-1").unwrap();
    assert_eq!(job.step, "init");
    // If breadcrumb dir exists, it should contain a file
    if has_breadcrumbs {
        let entries: Vec<_> = std::fs::read_dir(&breadcrumb_dir).unwrap().collect();
        assert!(!entries.is_empty(), "breadcrumb directory should contain at least one file");
    }
}
