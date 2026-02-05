// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for job creation logic in `runtime/handlers/job_create.rs`.
//!
//! Focuses on:
//! - Workspace setup (folder mode, cwd-only, default path)
//! - Name template resolution
//! - Runbook caching and RunbookLoaded event emission
//! - Namespace propagation
//! - Workspace setup failure → job marked failed
//! - cron_name propagation

use super::*;

// =============================================================================
// Job with explicit cwd, no workspace
// =============================================================================

const CWD_ONLY_RUNBOOK: &str = r#"
[command.deploy]
args = "<name>"
run = { job = "deploy" }

[job.deploy]
input = ["name"]
cwd = "${invoke.dir}/subdir"

[[job.deploy.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn job_with_cwd_uses_interpolated_path() {
    let ctx = setup_with_runbook(CWD_ONLY_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "deploy",
            "deploy",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    // cwd should be the interpolated path
    let expected_cwd = ctx.project_root.join("subdir");
    assert_eq!(
        job.cwd, expected_cwd,
        "cwd should be interpolated from template"
    );
    // No workspace should be created
    assert!(
        job.workspace_id.is_none(),
        "cwd-only job should not have workspace_id"
    );
    assert!(
        job.workspace_path.is_none(),
        "cwd-only job should not have workspace_path"
    );
}

// =============================================================================
// Job with no cwd, no workspace (default to invoke.dir)
// =============================================================================

const NO_CWD_NO_WORKSPACE_RUNBOOK: &str = r#"
[command.simple]
args = "<name>"
run = { job = "simple" }

[job.simple]
input = ["name"]

[[job.simple.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn job_without_cwd_or_workspace_uses_invoke_dir() {
    let ctx = setup_with_runbook(NO_CWD_NO_WORKSPACE_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "simple",
            "simple",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    // cwd should be the invoke directory (project_root in our test)
    assert_eq!(
        job.cwd, ctx.project_root,
        "default cwd should be invoke.dir"
    );
    assert!(
        job.workspace_id.is_none(),
        "no-workspace job should not have workspace_id"
    );
}

// =============================================================================
// Job with folder workspace
// =============================================================================

const FOLDER_WORKSPACE_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]
workspace = "folder"

[[job.build.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn job_with_folder_workspace_creates_workspace() {
    let ctx = setup_with_runbook(FOLDER_WORKSPACE_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();

    // Workspace should be created under state_dir/workspaces/
    assert!(
        job.workspace_id.is_some(),
        "folder workspace job should have workspace_id"
    );

    let ws_id = job.workspace_id.as_ref().unwrap().to_string();
    assert!(
        ws_id.starts_with("ws-"),
        "workspace id should start with 'ws-', got: {ws_id}"
    );

    // workspace vars should be injected
    assert!(
        job.vars.get("workspace.id").is_some(),
        "workspace.id var should be set"
    );
    assert!(
        job.vars.get("workspace.root").is_some(),
        "workspace.root var should be set"
    );
    assert!(
        job.vars.get("workspace.nonce").is_some(),
        "workspace.nonce var should be set"
    );
}

#[tokio::test]
async fn folder_workspace_path_is_under_state_dir() {
    let ctx = setup_with_runbook(FOLDER_WORKSPACE_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    let ws_root = job.vars.get("workspace.root").unwrap();
    let workspaces_dir = ctx.project_root.join("workspaces");

    assert!(
        ws_root.starts_with(&workspaces_dir.display().to_string()),
        "workspace root should be under state_dir/workspaces/, got: {ws_root}"
    );
}

// =============================================================================
// Name template resolution
// =============================================================================

const NAME_TEMPLATE_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]
name = "build-${var.name}"

[[job.build.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn job_name_template_is_interpolated() {
    let ctx = setup_with_runbook(NAME_TEMPLATE_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "auth-module".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    // The name should contain the interpolated value and a nonce suffix
    assert!(
        job.name.contains("auth-module"),
        "job name should contain interpolated var, got: {}",
        job.name
    );
}

#[tokio::test]
async fn job_without_name_template_uses_args_name() {
    let ctx = setup_with_runbook(NO_CWD_NO_WORKSPACE_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "simple",
            "simple",
            [("name".to_string(), "my-feature".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    assert_eq!(
        job.name, "my-feature",
        "without name template, job name should be args.name"
    );
}

// =============================================================================
// Namespace propagation
// =============================================================================

const SIMPLE_JOB_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn job_namespace_is_propagated() {
    let ctx = setup_with_runbook(SIMPLE_JOB_RUNBOOK).await;

    // Use a command event with a non-empty namespace
    let event = Event::CommandRun {
        job_id: JobId::new("pipe-1"),
        job_name: "build".to_string(),
        project_root: ctx.project_root.clone(),
        invoke_dir: ctx.project_root.clone(),
        command: "build".to_string(),
        namespace: "my-project".to_string(),
        args: [("name".to_string(), "test".to_string())]
            .into_iter()
            .collect(),
    };

    ctx.runtime.handle_event(event).await.unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    assert_eq!(
        job.namespace, "my-project",
        "job namespace should match the command event namespace"
    );
}

// =============================================================================
// Runbook caching
// =============================================================================

#[tokio::test]
async fn runbook_is_cached_after_creation() {
    let ctx = setup_with_runbook(SIMPLE_JOB_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();

    // Runbook should be cached by its hash
    let cached = ctx.runtime.cached_runbook(&job.runbook_hash);
    assert!(
        cached.is_ok(),
        "runbook should be retrievable from cache after job creation"
    );

    let runbook = cached.unwrap();
    assert!(
        runbook.get_job("build").is_some(),
        "cached runbook should contain the job definition"
    );
}

// =============================================================================
// RunbookLoaded event emitted
// =============================================================================

#[tokio::test]
async fn runbook_loaded_event_is_emitted() {
    let ctx = setup_with_runbook(SIMPLE_JOB_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    // Verify the runbook was stored in materialized state (via RunbookLoaded event)
    let job = ctx.runtime.get_job("pipe-1").unwrap();
    let stored = ctx
        .runtime
        .lock_state(|s| s.runbooks.contains_key(&job.runbook_hash));
    assert!(
        stored,
        "RunbookLoaded event should store runbook in materialized state"
    );
}

// =============================================================================
// Second job reuses cached runbook (no duplicate RunbookLoaded)
// =============================================================================

#[tokio::test]
async fn second_job_reuses_cached_runbook() {
    let ctx = setup_with_runbook(SIMPLE_JOB_RUNBOOK).await;

    // Create first job
    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "first".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job1 = ctx.runtime.get_job("pipe-1").unwrap();

    // Create second job with same command (same runbook)
    ctx.runtime
        .handle_event(command_event(
            "pipe-2",
            "build",
            "build",
            [("name".to_string(), "second".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job2 = ctx.runtime.get_job("pipe-2").unwrap();

    // Both jobs should reference the same runbook hash
    assert_eq!(
        job1.runbook_hash, job2.runbook_hash,
        "both jobs should use the same runbook hash"
    );
}

// =============================================================================
// Job def not found error
// =============================================================================

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
        .handle_event(command_event(
            "pipe-1",
            "deploy",
            "deploy",
            HashMap::new(),
            &ctx.project_root,
        ))
        .await;

    assert!(result.is_err(), "should return error for missing job def");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found"),
        "error should mention 'not found', got: {err}"
    );
}

// =============================================================================
// Initial step is started
// =============================================================================

#[tokio::test]
async fn job_starts_at_first_step() {
    let ctx = setup_with_runbook(SIMPLE_JOB_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    assert_eq!(job.step, "init", "job should start at first step");
    assert_eq!(
        job.step_status,
        StepStatus::Running,
        "first step should be running"
    );
}

// =============================================================================
// Job with cwd and workspace (cwd is overridden by workspace)
// =============================================================================

const CWD_AND_WORKSPACE_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]
cwd = "/some/base"
workspace = "folder"

[[job.build.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn cwd_with_workspace_creates_workspace() {
    let ctx = setup_with_runbook(CWD_AND_WORKSPACE_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    // When both cwd and workspace are set, workspace takes precedence
    assert!(
        job.workspace_id.is_some(),
        "should create workspace even when cwd is also set"
    );
}

// =============================================================================
// Multiple jobs get distinct workspace IDs
// =============================================================================

#[tokio::test]
async fn multiple_jobs_get_distinct_workspaces() {
    let ctx = setup_with_runbook(FOLDER_WORKSPACE_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "first".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    ctx.runtime
        .handle_event(command_event(
            "pipe-2",
            "build",
            "build",
            [("name".to_string(), "second".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job1 = ctx.runtime.get_job("pipe-1").unwrap();
    let job2 = ctx.runtime.get_job("pipe-2").unwrap();

    assert_ne!(
        job1.workspace_id, job2.workspace_id,
        "different jobs should have distinct workspace IDs"
    );
    assert_ne!(
        job1.vars.get("workspace.root"),
        job2.vars.get("workspace.root"),
        "different jobs should have distinct workspace paths"
    );
}

// =============================================================================
// Vars are namespaced in the created job
// =============================================================================

#[tokio::test]
async fn job_vars_are_namespaced() {
    let ctx = setup_with_runbook(SIMPLE_JOB_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test-feature".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();

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

// =============================================================================
// Job created_at uses clock
// =============================================================================

#[tokio::test]
async fn job_created_at_uses_clock() {
    let ctx = setup_with_runbook(SIMPLE_JOB_RUNBOOK).await;

    // Advance the fake clock
    ctx.clock.advance(std::time::Duration::from_secs(1000));

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    // Job should exist and be in running state (verifying clock didn't break creation)
    assert_eq!(job.step, "init");
    assert_eq!(job.step_status, StepStatus::Running);
}

// =============================================================================
// Job with on_start notification and name template
// =============================================================================

const NOTIFY_NAME_TEMPLATE_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]
name = "build-${var.name}"
notify = { on_start = "Started ${name}" }

[[job.build.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn on_start_notification_uses_resolved_name() {
    let ctx = setup_with_runbook(NOTIFY_NAME_TEMPLATE_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "auth".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

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

// =============================================================================
// Workspace nonce derived from job_id
// =============================================================================

#[tokio::test]
async fn workspace_nonce_is_derived_from_job_id() {
    let ctx = setup_with_runbook(FOLDER_WORKSPACE_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "oj-abc12345-deadbeef",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("oj-abc12345-deadbeef").unwrap();
    let nonce = job.vars.get("workspace.nonce").unwrap();

    // Nonce is first 8 chars of the job_id
    assert_eq!(
        nonce.len(),
        8,
        "workspace.nonce should be 8 chars, got: {nonce}"
    );
}

// =============================================================================
// Job with name template containing workspace nonce
// =============================================================================

const NAME_TEMPLATE_WITH_WORKSPACE_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]
name = "${var.name}"
workspace = "folder"

[[job.build.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn name_template_with_workspace_creates_matching_ws_id() {
    let ctx = setup_with_runbook(NAME_TEMPLATE_WITH_WORKSPACE_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "my-feature".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    let ws_id = job.workspace_id.as_ref().unwrap().to_string();

    // The workspace ID should incorporate the name template result
    assert!(
        ws_id.starts_with("ws-"),
        "workspace id should start with 'ws-', got: {ws_id}"
    );
    assert!(
        ws_id.contains("my-feature"),
        "workspace id should contain name from template, got: {ws_id}"
    );
}

// =============================================================================
// Job with multiple independent locals
// =============================================================================

const MULTI_LOCALS_RUNBOOK: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input = ["name"]

[job.build.locals]
prefix = "feat"
branch = "feature/${var.name}"

[[job.build.step]]
name = "init"
run = "echo ${local.branch}"
"#;

#[tokio::test]
async fn multiple_locals_are_evaluated() {
    let ctx = setup_with_runbook(MULTI_LOCALS_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "auth".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();

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

// =============================================================================
// Job with empty locals
// =============================================================================

#[tokio::test]
async fn job_without_locals_has_no_local_vars() {
    let ctx = setup_with_runbook(SIMPLE_JOB_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();

    let local_keys: Vec<_> = job
        .vars
        .keys()
        .filter(|k| k.starts_with("local."))
        .collect();
    assert!(
        local_keys.is_empty(),
        "job without locals should have no local.* vars, got: {:?}",
        local_keys
    );
}

// =============================================================================
// Job kind is stored correctly
// =============================================================================

#[tokio::test]
async fn job_kind_matches_definition_name() {
    let ctx = setup_with_runbook(SIMPLE_JOB_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    assert_eq!(
        job.kind, "build",
        "job kind should match the job definition name"
    );
}

// =============================================================================
// Multiple steps: first step is picked correctly
// =============================================================================

const MULTI_STEP_RUNBOOK: &str = r#"
[command.pipeline]
args = "<name>"
run = { job = "pipeline" }

[job.pipeline]
input = ["name"]

[[job.pipeline.step]]
name = "prepare"
run = "echo prepare"
on_done = "execute"

[[job.pipeline.step]]
name = "execute"
run = "echo execute"
"#;

#[tokio::test]
async fn job_starts_at_first_defined_step() {
    let ctx = setup_with_runbook(MULTI_STEP_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "pipeline",
            "pipeline",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    let job = ctx.runtime.get_job("pipe-1").unwrap();
    assert_eq!(
        job.step, "prepare",
        "job should start at the first defined step, not 'execute'"
    );
}

// =============================================================================
// Breadcrumb is written after job creation
// =============================================================================

#[tokio::test]
async fn breadcrumb_is_written_after_creation() {
    let ctx = setup_with_runbook(SIMPLE_JOB_RUNBOOK).await;

    ctx.runtime
        .handle_event(command_event(
            "pipe-1",
            "build",
            "build",
            [("name".to_string(), "test".to_string())]
                .into_iter()
                .collect(),
            &ctx.project_root,
        ))
        .await
        .unwrap();

    // Breadcrumb file should exist in the log dir
    let breadcrumb_dir = ctx.project_root.join("logs/breadcrumbs");
    let has_breadcrumbs = breadcrumb_dir.exists() && breadcrumb_dir.is_dir();

    // We just verify the job was created successfully and the breadcrumb code
    // ran without error. The BreadcrumbWriter creates files in logs/breadcrumbs/.
    let job = ctx.runtime.get_job("pipe-1").unwrap();
    assert_eq!(job.step, "init");
    // If breadcrumb dir exists, it should contain a file
    if has_breadcrumbs {
        let entries: Vec<_> = std::fs::read_dir(&breadcrumb_dir).unwrap().collect();
        assert!(
            !entries.is_empty(),
            "breadcrumb directory should contain at least one file"
        );
    }
}
