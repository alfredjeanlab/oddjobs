// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj run <command> [args]` - Run a command from the runbook

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use clap::Args;
use oj_runbook::RunDirective;

use crate::client::DaemonClient;

#[derive(Args)]
pub struct RunArgs {
    /// Command to run (e.g., "build")
    pub command: Option<String>,

    /// Positional arguments for the command
    #[arg(trailing_var_arg = true)]
    pub args: Vec<String>,

    /// Runbook file to load (e.g., "build.toml")
    #[arg(long = "runbook")]
    pub runbook: Option<String>,
}

/// Quick validation that a command exists in the runbook.
/// This is a lightweight check before sending to daemon.
/// Returns the loaded runbook on success for arg validation.
pub fn prevalidate_command(
    project_root: &Path,
    command: &str,
    runbook_file: Option<&str>,
) -> Result<oj_runbook::Runbook> {
    let runbook = crate::load_runbook(project_root, command, runbook_file)?;
    if runbook.get_command(command).is_none() {
        bail!("unknown command: {}", command);
    }
    Ok(runbook)
}

fn print_command_help(
    project_root: &Path,
    command: &str,
    runbook_file: Option<&str>,
) -> Result<()> {
    let runbook = crate::load_runbook(project_root, command, runbook_file)?;
    let cmd_def = runbook
        .get_command(command)
        .ok_or_else(|| anyhow::anyhow!("unknown command: {}", command))?;

    let runbook_dir = project_root.join(".oj/runbooks");
    let comment = oj_runbook::find_command_with_comment(&runbook_dir, command)
        .ok()
        .flatten()
        .and_then(|(_, comment)| comment);

    eprint!("{}", cmd_def.format_help(command, comment.as_ref()));
    std::process::exit(0);
}

fn print_available_commands(project_root: &Path) -> Result<()> {
    let runbook_dir = project_root.join(".oj/runbooks");
    let commands = oj_runbook::collect_all_commands(&runbook_dir).unwrap_or_default();

    let mut help = crate::color::HelpPrinter::new();
    format_available_commands(&mut help, &commands);
    print!("{}", help.finish());
    Ok(())
}

fn format_available_commands(
    help: &mut crate::color::HelpPrinter,
    commands: &[(String, oj_runbook::CommandDef)],
) {
    help.usage("oj run <COMMAND> [ARGS]...");
    help.blank();

    if commands.is_empty() {
        help.plain("No commands found.");
        help.plain("Define commands in .oj/runbooks/*.hcl");
    } else {
        help.header("Commands:");
        for (name, cmd) in commands {
            let args_str = cmd.args.usage_line();
            let line = if args_str.is_empty() {
                name.to_string()
            } else {
                format!("{name} {args_str}")
            };
            help.entry(&line, 40, cmd.description.as_deref());
        }
    }

    help.blank();
    help.hint("For more information, try '--help'.");
}

pub async fn handle(
    args: RunArgs,
    project_root: &Path,
    invoke_dir: &Path,
    namespace: &str,
) -> Result<()> {
    let Some(ref command) = args.command else {
        return print_available_commands(project_root);
    };

    // Check for --help before anything else
    if args.args.iter().any(|a| a == "--help" || a == "-h") {
        return print_command_help(project_root, command, args.runbook.as_deref());
    }

    // Prevalidate locally for fast feedback
    let runbook = prevalidate_command(project_root, command, args.runbook.as_deref())?;
    // Safe: prevalidate_command already verified the command exists
    let Some(cmd_def) = runbook.get_command(command) else {
        bail!("command not found: {}", command);
    };

    // Split raw args into positional and named based on command's ArgSpec
    let (positional, named) = cmd_def.args.split_raw_args(&args.args);

    cmd_def.validate_args(&positional, &named)?;

    // Shell directives execute locally
    if let RunDirective::Shell(ref cmd) = cmd_def.run {
        return execute_shell_inline(
            cmd,
            cmd_def,
            &positional,
            &named,
            project_root,
            invoke_dir,
            namespace,
        );
    }

    // Pipeline or Agent: dispatch to daemon
    let client = DaemonClient::for_action()?;
    dispatch_to_daemon(
        &client,
        project_root,
        invoke_dir,
        namespace,
        command,
        &positional,
        &named,
    )
    .await
}

fn execute_shell_inline(
    cmd: &str,
    cmd_def: &oj_runbook::CommandDef,
    positional: &[String],
    named: &HashMap<String, String>,
    project_root: &Path,
    invoke_dir: &Path,
    namespace: &str,
) -> Result<()> {
    let parsed_args = cmd_def.parse_args(positional, named);
    let mut vars: HashMap<String, String> = parsed_args
        .iter()
        .map(|(k, v)| (format!("args.{}", k), v.clone()))
        .collect();
    vars.insert("invoke.dir".to_string(), invoke_dir.display().to_string());
    vars.insert("workspace".to_string(), project_root.display().to_string());

    let interpolated = oj_runbook::interpolate_shell(cmd, &vars);

    let wrapped = format!("set -euo pipefail\n{interpolated}");
    let status = std::process::Command::new("bash")
        .arg("-c")
        .arg(&wrapped)
        .current_dir(project_root)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .env("OJ_NAMESPACE", namespace)
        .status()?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        std::process::exit(code);
    }

    Ok(())
}

async fn dispatch_to_daemon(
    client: &DaemonClient,
    project_root: &Path,
    invoke_dir: &Path,
    namespace: &str,
    command: &str,
    positional: &[String],
    named: &HashMap<String, String>,
) -> Result<()> {
    let result = client
        .run_command(
            project_root,
            invoke_dir,
            namespace,
            command,
            positional,
            named,
        )
        .await?;

    match result {
        crate::client::RunCommandResult::Pipeline {
            pipeline_id,
            pipeline_name,
        } => dispatch_pipeline(client, namespace, command, &pipeline_id, &pipeline_name).await,
        crate::client::RunCommandResult::AgentRun {
            agent_run_id,
            agent_name,
        } => {
            dispatch_agent_run(namespace, command, &agent_run_id, &agent_name);
            Ok(())
        }
    }
}

async fn dispatch_pipeline(
    client: &DaemonClient,
    namespace: &str,
    command: &str,
    pipeline_id: &str,
    pipeline_name: &str,
) -> Result<()> {
    let short_id = &pipeline_id[..8.min(pipeline_id.len())];
    println!("Project: {namespace}");
    println!("Command {} invoked.", command);
    println!("Waiting for pipeline to start... (Ctrl+C to skip)");
    println!();

    // Poll for pipeline start
    let wait_ms = std::env::var("OJ_RUN_WAIT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(10_000);
    let poll_interval = Duration::from_millis(500);
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    let mut started = false;

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                break;
            }
            _ = tokio::time::sleep(poll_interval) => {
                if Instant::now() >= deadline {
                    break;
                }
                if let Ok(Some(p)) = client.get_pipeline(pipeline_id).await {
                    if p.step_status != "Pending" {
                        started = true;
                        break;
                    }
                }
            }
        }
    }

    if started {
        println!("Started {} (pipeline: {})", pipeline_name, short_id);
    } else {
        println!("Still waiting for the pipeline to start, check:");
    }
    super::pipeline::print_pipeline_commands(short_id);

    Ok(())
}

fn dispatch_agent_run(namespace: &str, command: &str, agent_run_id: &str, agent_name: &str) {
    let short_id = &agent_run_id[..8.min(agent_run_id.len())];
    println!("Project: {namespace}");
    println!("Command {} invoked.", command);
    println!("Agent: {agent_name} ({short_id})");
    println!();
    println!("  oj agent show {short_id}");
    println!("  oj agent logs {short_id}");
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
