// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Core AST-walking execution logic.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Instant;

use tokio::io::AsyncWriteExt;

use crate::{
    AndOrList, BraceGroup, Command, CommandItem, CommandList, LogicalOp, Job, SimpleCommand,
    Span, Subshell,
};

use super::error::ExecError;
use super::expand;
use super::redirect;
use super::result::{CommandTrace, ExecOutput};

// ---------------------------------------------------------------------------
// Execution context
// ---------------------------------------------------------------------------

/// Shared execution context threaded through recursive calls.
#[derive(Clone)]
pub(crate) struct ExecContext {
    pub(crate) cwd: PathBuf,
    pub(crate) env: HashMap<String, String>,
    pub(crate) variables: HashMap<String, String>,
    pub(crate) snippet_limit: usize,
    pub(crate) pipefail: bool,
    /// IFS (Internal Field Separator) for word splitting.
    /// Default: " \t\n" (space, tab, newline).
    pub(crate) ifs: String,
    /// Exit code of the last executed command (for $?).
    pub(crate) last_exit_code: i32,
}

// ---------------------------------------------------------------------------
// Command list (top-level, fail-fast)
// ---------------------------------------------------------------------------

/// Execute a full command list with fail-fast (`set -e`) semantics.
///
/// Returns a boxed future to support async recursion (command substitution
/// can re-enter this function).
pub(crate) fn execute_command_list<'a>(
    ctx: &'a mut ExecContext,
    cmd_list: &'a CommandList,
) -> Pin<Box<dyn Future<Output = Result<ExecOutput, ExecError>> + 'a>> {
    Box::pin(async move {
        let mut traces = Vec::new();
        let mut last_exit = 0;

        for and_or in &cmd_list.commands {
            let (exit_code, mut cmd_traces) = execute_and_or_list(ctx, and_or).await?;
            last_exit = exit_code;

            // Extract the failing command name from traces for the error message.
            let failing_cmd = cmd_traces
                .last()
                .map(|t| t.command.clone())
                .unwrap_or_default();

            traces.append(&mut cmd_traces);

            if exit_code != 0 {
                return Err(ExecError::CommandFailed {
                    command: failing_cmd,
                    exit_code,
                    span: and_or.span,
                });
            }
        }

        Ok(ExecOutput {
            exit_code: last_exit,
            traces,
        })
    })
}

/// Execute a command list and return the captured stdout as a string.
///
/// Used for command substitution — runs with unlimited snippet capture so
/// the full output is available.
pub(crate) async fn execute_command_list_capture(
    ctx: &mut ExecContext,
    cmd_list: &CommandList,
) -> Result<String, ExecError> {
    let saved_limit = ctx.snippet_limit;
    ctx.snippet_limit = usize::MAX;
    let result = execute_command_list(ctx, cmd_list).await;
    ctx.snippet_limit = saved_limit;

    let output = result?;
    let mut captured = String::new();
    for trace in &output.traces {
        if let Some(ref s) = trace.stdout_snippet {
            captured.push_str(s);
        }
    }
    Ok(captured)
}

// ---------------------------------------------------------------------------
// AND / OR chains
// ---------------------------------------------------------------------------

/// Execute an AND/OR chain with short-circuit evaluation.
async fn execute_and_or_list(
    ctx: &mut ExecContext,
    and_or: &AndOrList,
) -> Result<(i32, Vec<CommandTrace>), ExecError> {
    let mut all_traces = Vec::new();

    let (mut last_exit, mut traces) = execute_command_item(ctx, &and_or.first).await?;
    ctx.last_exit_code = last_exit; // Track exit code for $?
    all_traces.append(&mut traces);

    for (op, next_item) in &and_or.rest {
        let should_run = match op {
            LogicalOp::And => last_exit == 0,
            LogicalOp::Or => last_exit != 0,
        };
        if should_run {
            let (exit_code, mut traces) = execute_command_item(ctx, next_item).await?;
            all_traces.append(&mut traces);
            last_exit = exit_code;
            ctx.last_exit_code = exit_code; // Track exit code for $?
        }
    }

    Ok((last_exit, all_traces))
}

// ---------------------------------------------------------------------------
// Command item dispatch
// ---------------------------------------------------------------------------

/// Execute a single command item (dispatches to simple/job/subshell/brace).
async fn execute_command_item(
    ctx: &mut ExecContext,
    item: &CommandItem,
) -> Result<(i32, Vec<CommandTrace>), ExecError> {
    if item.background {
        return Err(ExecError::Unsupported {
            feature: "background execution (&)".to_string(),
            span: item.span,
        });
    }

    match &item.command {
        Command::Simple(cmd) => {
            let (exit_code, trace) = execute_simple_command(ctx, cmd).await?;
            Ok((exit_code, vec![trace]))
        }
        Command::Job(job) => execute_job(ctx, job).await,
        Command::Subshell(subshell) => execute_subshell(ctx, subshell).await,
        Command::BraceGroup(group) => execute_brace_group(ctx, group).await,
    }
}

// ---------------------------------------------------------------------------
// Simple command
// ---------------------------------------------------------------------------

/// Spawn a simple command via `tokio::process::Command`.
async fn execute_simple_command(
    ctx: &mut ExecContext,
    cmd: &SimpleCommand,
) -> Result<(i32, CommandTrace), ExecError> {
    let start = Instant::now();

    // Expand name, args, and env assignments.
    // Command name: still use expand_word (name cannot be multiple words)
    let expanded_name = expand::expand_word(ctx, &cmd.name).await?;
    // Arguments: use expand_word_split_glob (word splitting + glob expansion)
    let glob_config = super::expand_glob::GlobConfig::default();
    let mut expanded_args: Vec<String> = Vec::new();
    for arg in &cmd.args {
        let fields = expand::expand_word_split_glob(ctx, arg, &glob_config).await?;
        expanded_args.extend(fields);
    }
    let mut expanded_env: Vec<(String, String)> = Vec::new();
    for ea in &cmd.env {
        let val = expand::expand_word(ctx, &ea.value).await?;
        expanded_env.push((ea.name.clone(), val));
    }

    // Handle assignment-only commands (VAR=value without a command).
    // These set shell variables rather than spawning a process.
    if expanded_name.is_empty() {
        for (name, value) in expanded_env {
            ctx.variables.insert(name, value);
        }
        let duration = start.elapsed();
        let trace = CommandTrace {
            command: String::new(),
            args: Vec::new(),
            exit_code: 0,
            duration,
            stdout_snippet: None,
            stderr_snippet: None,
            span: cmd.span,
        };
        return Ok((0, trace));
    }

    // Tracing span.
    let cmd_span = tracing::info_span!(
        "shell.cmd",
        cmd = %expanded_name,
        args = ?expanded_args,
        exit_code = tracing::field::Empty,
        duration_ms = tracing::field::Empty,
    );

    // Build process command.
    let mut process = tokio::process::Command::new(&expanded_name);
    process.args(&expanded_args);
    process.current_dir(&ctx.cwd);
    process.envs(&ctx.env);
    for (k, v) in &expanded_env {
        process.env(k, v);
    }

    // Default: pipe stdout/stderr for capture.
    process.stdout(std::process::Stdio::piped());
    process.stderr(std::process::Stdio::piped());

    // Apply redirections (may override defaults above).
    let applied = redirect::apply_redirections(&mut process, &cmd.redirections, ctx).await?;

    // Spawn.
    let mut child = process.spawn().map_err(|source| ExecError::SpawnFailed {
        command: expanded_name.clone(),
        source,
        span: cmd.span,
    })?;

    // Write heredoc / herestring data to stdin.
    if let Some(stdin_data) = applied.stdin_data {
        if let Some(mut stdin) = child.stdin.take() {
            let write_result = stdin.write_all(&stdin_data).await;
            drop(stdin); // close pipe to signal EOF
            write_result.map_err(|source| ExecError::SpawnFailed {
                command: expanded_name.clone(),
                source,
                span: cmd.span,
            })?;
        }
    }

    // Wait for completion.
    let output = child
        .wait_with_output()
        .await
        .map_err(|source| ExecError::SpawnFailed {
            command: expanded_name.clone(),
            source,
            span: cmd.span,
        })?;

    let duration = start.elapsed();
    let exit_code = output.status.code().unwrap_or(-1);

    // Record tracing fields.
    cmd_span.record("exit_code", exit_code);
    cmd_span.record("duration_ms", duration.as_millis() as u64);

    let trace = CommandTrace {
        command: expanded_name,
        args: expanded_args,
        exit_code,
        duration,
        stdout_snippet: truncate_snippet(&output.stdout, ctx.snippet_limit),
        stderr_snippet: truncate_snippet(&output.stderr, ctx.snippet_limit),
        span: cmd.span,
    };

    Ok((exit_code, trace))
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

/// Execute a job, wiring stdout→stdin between stages.
///
/// All job stages are spawned first, then connected via async relay
/// tasks that copy data between `ChildStdout` and `ChildStdin`. This avoids
/// requiring unsafe fd conversion while still providing true job
/// concurrency.
async fn execute_job(
    ctx: &mut ExecContext,
    job: &Job,
) -> Result<(i32, Vec<CommandTrace>), ExecError> {
    let _job_span = tracing::info_span!("shell.job").entered();

    let n = job.commands.len();
    if n == 0 {
        return Ok((0, Vec::new()));
    }

    // Phase 1: expand all commands (sequentially, may mutate variables via :=).
    struct ExpandedCmd {
        name: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        span: Span,
        redirections_idx: usize,
    }

    let glob_config = super::expand_glob::GlobConfig::default();
    let mut expanded = Vec::with_capacity(n);
    for (i, cmd) in job.commands.iter().enumerate() {
        let name = expand::expand_word(ctx, &cmd.name).await?;
        let mut args = Vec::new();
        for arg in &cmd.args {
            let fields = expand::expand_word_split_glob(ctx, arg, &glob_config).await?;
            args.extend(fields);
        }
        let mut env = Vec::new();
        for ea in &cmd.env {
            let val = expand::expand_word(ctx, &ea.value).await?;
            env.push((ea.name.clone(), val));
        }
        expanded.push(ExpandedCmd {
            name,
            args,
            env,
            span: cmd.span,
            redirections_idx: i,
        });
    }

    // Phase 2: spawn all commands.
    //
    // Stdin for commands 1..N-1 is piped (so we can relay from the previous
    // command's stdout).  All commands get piped stdout and stderr.
    struct SpawnedChild {
        child: tokio::process::Child,
        name: String,
        args: Vec<String>,
        span: Span,
        start: Instant,
    }

    let mut spawned: Vec<SpawnedChild> = Vec::with_capacity(n);

    for (i, ec) in expanded.iter().enumerate() {
        let start = Instant::now();
        let mut process = tokio::process::Command::new(&ec.name);
        process.args(&ec.args);
        process.current_dir(&ctx.cwd);
        process.envs(&ctx.env);
        for (k, v) in &ec.env {
            process.env(k, v);
        }

        // Commands after the first receive input via piped stdin.
        if i > 0 {
            process.stdin(std::process::Stdio::piped());
        }

        // All commands get piped stdout and stderr.
        process.stdout(std::process::Stdio::piped());
        process.stderr(std::process::Stdio::piped());

        // Apply redirections (may override the defaults above).
        let applied = redirect::apply_redirections(
            &mut process,
            &job.commands[ec.redirections_idx].redirections,
            ctx,
        )
        .await?;

        let mut child = process.spawn().map_err(|source| ExecError::SpawnFailed {
            command: ec.name.clone(),
            source,
            span: ec.span,
        })?;

        // Heredoc / herestring data.
        if let Some(stdin_data) = applied.stdin_data {
            if let Some(mut stdin) = child.stdin.take() {
                let write_result = stdin.write_all(&stdin_data).await;
                drop(stdin);
                write_result.map_err(|source| ExecError::SpawnFailed {
                    command: ec.name.clone(),
                    source,
                    span: ec.span,
                })?;
            }
        }

        spawned.push(SpawnedChild {
            child,
            name: ec.name.clone(),
            args: ec.args.clone(),
            span: ec.span,
            start,
        });
    }

    // Phase 3: wire adjacent commands via async relay tasks.
    //
    // For each adjacent pair (i, i+1), spawn a task that copies bytes from
    // child[i].stdout to child[i+1].stdin.
    let mut relay_tasks = Vec::with_capacity(n.saturating_sub(1));
    for i in 0..n.saturating_sub(1) {
        let stdout = spawned[i].child.stdout.take();
        let stdin = spawned[i + 1].child.stdin.take();
        if let (Some(mut reader), Some(mut writer)) = (stdout, stdin) {
            relay_tasks.push(tokio::spawn(async move {
                let _ = tokio::io::copy(&mut reader, &mut writer).await;
            }));
        }
    }

    // Phase 4: wait for all children concurrently.
    let snippet_limit = ctx.snippet_limit;
    let mut join_handles = Vec::with_capacity(n);

    for sc in spawned {
        join_handles.push(tokio::spawn(async move {
            let output =
                sc.child
                    .wait_with_output()
                    .await
                    .map_err(|source| ExecError::SpawnFailed {
                        command: sc.name.clone(),
                        source,
                        span: sc.span,
                    })?;
            let duration = sc.start.elapsed();
            let exit_code = output.status.code().unwrap_or(-1);
            Ok::<CommandTrace, ExecError>(CommandTrace {
                command: sc.name,
                args: sc.args,
                exit_code,
                duration,
                stdout_snippet: truncate_snippet(&output.stdout, snippet_limit),
                stderr_snippet: truncate_snippet(&output.stderr, snippet_limit),
                span: sc.span,
            })
        }));
    }

    let mut traces = Vec::with_capacity(n);
    let mut last_exit = 0;
    let mut rightmost_failure: Option<i32> = None;

    for handle in join_handles {
        let trace = handle.await.map_err(|e| ExecError::SpawnFailed {
            command: String::new(),
            source: std::io::Error::other(e),
            span: job.span,
        })??;

        last_exit = trace.exit_code;
        if trace.exit_code != 0 {
            rightmost_failure = Some(trace.exit_code);
        }
        traces.push(trace);
    }

    let exit_code = if ctx.pipefail {
        rightmost_failure.unwrap_or(0)
    } else {
        last_exit
    };

    // Wait for relay tasks to finish.
    for task in relay_tasks {
        let _ = task.await;
    }

    Ok((exit_code, traces))
}

// ---------------------------------------------------------------------------
// Subshells and brace groups
// ---------------------------------------------------------------------------

/// Execute a subshell — clones the context so variable changes don't escape.
async fn execute_subshell(
    ctx: &mut ExecContext,
    subshell: &Subshell,
) -> Result<(i32, Vec<CommandTrace>), ExecError> {
    let mut sub_ctx = ctx.clone();
    let (exit_code, mut traces) = match execute_command_list(&mut sub_ctx, &subshell.body).await {
        Ok(output) => (output.exit_code, output.traces),
        Err(ExecError::CommandFailed { exit_code, .. }) => {
            // In a subshell, a failing command produces a non-zero exit —
            // it does NOT propagate as a hard error (the parent decides).
            (exit_code, Vec::new())
        }
        Err(e) => return Err(e),
    };

    // Apply redirections to captured output.
    if !subshell.redirections.is_empty() {
        // Collect stdout/stderr from all traces.
        let mut stdout = String::new();
        let mut stderr = String::new();
        for trace in &traces {
            if let Some(ref s) = trace.stdout_snippet {
                stdout.push_str(s);
            }
            if let Some(ref s) = trace.stderr_snippet {
                stderr.push_str(s);
            }
        }

        let result =
            redirect::apply_captured_redirections(&subshell.redirections, &stdout, &stderr, ctx)
                .await?;

        // Clear redirected output from traces (it was written to file).
        if result.stdout_redirected || result.stderr_redirected {
            for trace in &mut traces {
                if result.stdout_redirected {
                    trace.stdout_snippet = None;
                }
                if result.stderr_redirected {
                    trace.stderr_snippet = None;
                }
            }
        }
    }

    Ok((exit_code, traces))
}

/// Execute a brace group — shares the parent context.
async fn execute_brace_group(
    ctx: &mut ExecContext,
    group: &BraceGroup,
) -> Result<(i32, Vec<CommandTrace>), ExecError> {
    let (exit_code, mut traces) = match execute_command_list(ctx, &group.body).await {
        Ok(output) => (output.exit_code, output.traces),
        Err(ExecError::CommandFailed { exit_code, .. }) => (exit_code, Vec::new()),
        Err(e) => return Err(e),
    };

    // Apply redirections to captured output.
    if !group.redirections.is_empty() {
        // Collect stdout/stderr from all traces.
        let mut stdout = String::new();
        let mut stderr = String::new();
        for trace in &traces {
            if let Some(ref s) = trace.stdout_snippet {
                stdout.push_str(s);
            }
            if let Some(ref s) = trace.stderr_snippet {
                stderr.push_str(s);
            }
        }

        let result =
            redirect::apply_captured_redirections(&group.redirections, &stdout, &stderr, ctx)
                .await?;

        // Clear redirected output from traces (it was written to file).
        if result.stdout_redirected || result.stderr_redirected {
            for trace in &mut traces {
                if result.stdout_redirected {
                    trace.stdout_snippet = None;
                }
                if result.stderr_redirected {
                    trace.stderr_snippet = None;
                }
            }
        }
    }

    Ok((exit_code, traces))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Truncate a byte buffer to a UTF-8–safe snippet of at most `limit` bytes.
fn truncate_snippet(bytes: &[u8], limit: usize) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let s = String::from_utf8_lossy(bytes);
    if s.len() <= limit {
        Some(s.into_owned())
    } else {
        let mut end = limit.min(s.len());
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        Some(s[..end].to_string())
    }
}
