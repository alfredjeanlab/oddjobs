// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Run directive tests

use super::*;

/// Runbook with a command that uses shell run directive
const RUNBOOK_SHELL_COMMAND: &str = r#"
[command.shell_cmd]
args = "<name>"
run = "echo hello"

[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn command_with_shell_directive_creates_job() {
    let ctx = setup_with_runbook(RUNBOOK_SHELL_COMMAND).await;

    handle_event_chain(
        &ctx,
        command_event("job-1", "build", "shell_cmd", vars!("name" => "test"), &ctx.project_path),
    )
    .await;

    // Job should be created with kind = command name
    let job = ctx.runtime.get_job("job-1").unwrap();
    assert_eq!(job.kind, "shell_cmd");
    assert_eq!(job.step, "run");
}

#[tokio::test]
async fn command_with_shell_directive_completes_on_exit() {
    let mut ctx = setup_with_runbook(RUNBOOK_SHELL_COMMAND).await;

    handle_event_chain(
        &ctx,
        command_event("job-1", "build", "shell_cmd", vars!("name" => "test"), &ctx.project_path),
    )
    .await;

    // Shell runs async - wait for ShellExited event
    let event = ctx.event_rx.recv().await.unwrap();
    assert!(matches!(event, Event::ShellExited { exit_code: 0, .. }));

    // Process the ShellExited event - job should auto-complete (no next step)
    ctx.runtime.handle_event(event).await.unwrap();

    let job = ctx.runtime.get_job("job-1").unwrap();
    assert_eq!(job.step, "done");
    assert!(job.is_terminal());
}

/// Runbook with a command that uses args.* project interpolation in shell directive
const RUNBOOK_SHELL_ARGS_NAMESPACE: &str = r#"
[command.file_bug]
args = "<description>"
run = "test '${args.description}' = 'button broken'"

[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = "echo init"
"#;

#[tokio::test]
async fn command_shell_directive_interpolates_args_namespace() {
    let mut ctx = setup_with_runbook(RUNBOOK_SHELL_ARGS_NAMESPACE).await;

    handle_event_chain(
        &ctx,
        command_event(
            "job-1",
            "build",
            "file_bug",
            vars!("description" => "button broken"),
            &ctx.project_path,
        ),
    )
    .await;

    // The shell command `test '${args.description}' = 'button broken'` should succeed
    // (exit 0) only if args.description was interpolated to "button broken".
    // If interpolation fails, the literal text won't match and exit code will be non-zero.
    let event = ctx.event_rx.recv().await.unwrap();
    assert!(
        matches!(event, Event::ShellExited { exit_code: 0, .. }),
        "expected exit_code 0 (args.* interpolated), got: {event:?}"
    );
}

/// Runbook with a command that uses input.* project is now rejected at parse time.
/// The parser validates that command.run does not use job-only namespaces.
/// See crates/runbook/src/parser_tests for parse-time validation tests.
///
/// Previously this was a runtime test that checked ${input.*} wasn't interpolated;
/// now the runbook parser rejects it outright with a helpful error message.

#[tokio::test]
async fn command_with_agent_directive_spawns_standalone_agent() {
    let ctx = setup_with_runbook(&test_runbook_agent("")).await;

    let result = ctx
        .runtime
        .handle_event(crew_command_event(
            "job-1",
            "worker",
            "agent_cmd",
            vars!("name" => "test"),
            &ctx.project_path,
        ))
        .await;

    assert!(result.is_ok(), "agent directive should succeed: {:?}", result.err());
    let events = result.unwrap();
    // Should produce at least RunbookLoaded and CrewCreated events
    assert!(
        events.iter().any(|e| matches!(e, Event::CrewCreated { .. })),
        "expected CrewCreated event, got: {:?}",
        events
    );
}

#[tokio::test]
async fn command_agent_max_concurrency_error() {
    let ctx = setup_with_runbook(&test_runbook_agent("max_concurrency = 1")).await;

    // First spawn should succeed
    let result = ctx
        .runtime
        .handle_event(crew_command_event(
            "job-1",
            "worker",
            "agent_cmd",
            vars!("name" => "test"),
            &ctx.project_path,
        ))
        .await;
    assert!(result.is_ok(), "first agent spawn should succeed: {:?}", result.err());

    // Second spawn should fail due to max_concurrency=1
    let result = ctx
        .runtime
        .handle_event(crew_command_event(
            "job-2",
            "worker",
            "agent_cmd",
            vars!("name" => "test2"),
            &ctx.project_path,
        ))
        .await;
    assert!(result.is_err(), "second spawn should fail");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("max concurrency"), "error should mention max concurrency, got: {}", err);
}

/// Runbook with a step that uses job run directive
const RUNBOOK_JOB_STEP: &str = r#"
[command.build]
args = "<name>"
run = { job = "build" }

[job.build]
input  = ["name"]

[[job.build.step]]
name = "init"
run = { job = "nested" }

[job.nested]
input  = []

[[job.nested.step]]
name = "init"
run = "echo nested"
"#;

#[tokio::test]
async fn step_with_job_directive_errors() {
    let ctx = setup_with_runbook(RUNBOOK_JOB_STEP).await;

    // CommandRun creates the job; the error happens when JobCreated
    // triggers start_step which rejects the nested job directive.
    let events = ctx
        .runtime
        .handle_event(command_event(
            "job-1",
            "build",
            "build",
            vars!("name" => "test"),
            &ctx.project_path,
        ))
        .await
        .unwrap();

    // Process the JobCreated event — start_step should fail
    let job_created = events
        .into_iter()
        .find(|e| matches!(e, Event::JobCreated { .. }))
        .expect("should have JobCreated event");
    let result = ctx.runtime.handle_event(job_created).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("nested job"));
}
