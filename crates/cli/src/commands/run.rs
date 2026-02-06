// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj run <command> [args]` - Run a command from the runbook

use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Result};
use clap::Args;
use oj_core::ShortId;
use oj_runbook::RunDirective;

use crate::client::DaemonClient;
use crate::color;

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

    /// Attach to the agent's tmux session after starting
    #[arg(long = "attach", conflicts_with = "no_attach")]
    pub attach: bool,

    /// Do not attach to the agent's tmux session
    #[arg(long = "no-attach", conflicts_with = "attach")]
    pub no_attach: bool,
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

/// Build the help text for `oj run` showing available runbook commands.
pub fn available_commands_help(project_root: &Path) -> String {
    let runbook_dir = project_root.join(".oj/runbooks");
    let commands = oj_runbook::collect_all_commands(&runbook_dir).unwrap_or_default();
    let warnings = oj_runbook::runbook_parse_warnings(&runbook_dir);
    let mut help = crate::color::HelpPrinter::new();
    format_available_commands(&mut help, &commands, &warnings);
    help.finish()
}

fn print_available_commands(project_root: &Path) -> Result<()> {
    print!("{}", available_commands_help(project_root));
    Ok(())
}

fn format_available_commands(
    help: &mut crate::color::HelpPrinter,
    commands: &[(String, oj_runbook::CommandDef)],
    warnings: &[String],
) {
    help.usage("oj run <COMMAND>");
    help.blank();

    if commands.is_empty() {
        help.plain("No commands found.");
        help.plain("Define commands in .oj/runbooks/*.hcl");
    } else {
        help.header("Commands:");
        for (name, cmd) in commands {
            help.entry(name, 20, cmd.description.as_deref());
        }
    }

    if !warnings.is_empty() {
        help.blank();
        help.header("Warnings:");
        for warning in warnings {
            help.plain(&format!("  {warning}"));
        }
    }

    help.blank();
    help.hint("For more information, try 'oj run <COMMAND> -h'.");
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

    // Extract --attach/--no-attach from trailing args (since trailing_var_arg
    // is greedy, these flags may end up in args.args rather than clap fields)
    let mut raw_args = args.args.clone();
    let mut cli_attach = None;
    raw_args.retain(|a| {
        if a == "--attach" {
            cli_attach = Some(true);
            false
        } else if a == "--no-attach" {
            cli_attach = Some(false);
            false
        } else {
            true
        }
    });
    let attach_override = if args.attach {
        Some(true)
    } else if args.no_attach {
        Some(false)
    } else {
        cli_attach
    };

    // Check for --help before anything else
    if raw_args.iter().any(|a| a == "--help" || a == "-h") {
        return print_command_help(project_root, command, args.runbook.as_deref());
    }

    // Prevalidate locally for fast feedback
    let runbook = prevalidate_command(project_root, command, args.runbook.as_deref())?;
    // Safe: prevalidate_command already verified the command exists
    let Some(cmd_def) = runbook.get_command(command) else {
        bail!("command not found: {}", command);
    };

    // Split raw args into positional and named based on command's ArgSpec
    let (positional, named) = cmd_def.args.split_raw_args(&raw_args);

    cmd_def.validate_args(&positional, &named)?;

    // Shell directives execute locally
    if let RunDirective::Shell(ref cmd) = cmd_def.run {
        return execute_shell_inline(cmd, cmd_def, &positional, &named, project_root, invoke_dir);
    }

    // Resolve attach preference: CLI override > runbook default > false
    let should_attach = attach_override.or(cmd_def.run.attach()).unwrap_or(false);

    // Job or Agent: dispatch to daemon
    let client = DaemonClient::for_action()?;
    let result = client
        .run_command(
            project_root,
            invoke_dir,
            namespace,
            command,
            &positional,
            &named,
        )
        .await?;

    match result {
        crate::client::RunCommandResult::Job { job_id, job_name } => {
            dispatch_job(&client, namespace, command, &job_id, &job_name).await
        }
        crate::client::RunCommandResult::AgentRun {
            agent_run_id,
            agent_name,
        } => {
            dispatch_agent_run(
                &client,
                namespace,
                command,
                &agent_run_id,
                &agent_name,
                should_attach,
            )
            .await
        }
    }
}

fn execute_shell_inline(
    cmd: &str,
    cmd_def: &oj_runbook::CommandDef,
    positional: &[String],
    named: &HashMap<String, String>,
    project_root: &Path,
    invoke_dir: &Path,
) -> Result<()> {
    let parsed_args = cmd_def.parse_args(positional, named);
    let mut vars: HashMap<String, String> = parsed_args
        .iter()
        .map(|(k, v)| (format!("args.{}", k), v.clone()))
        .collect();
    vars.insert("invoke.dir".to_string(), invoke_dir.display().to_string());
    vars.insert("workspace".to_string(), project_root.display().to_string());

    let interpolated = oj_runbook::interpolate_shell(cmd, &vars);

    // OJ_NAMESPACE is NOT injected here — the CLI shell-inline path runs
    // outside engine-managed workspaces.  If OJ_NAMESPACE is already set
    // in the parent environment (e.g. from a parent workspace), the child
    // process inherits it automatically via std::process::Command.
    let wrapped = format!("set -euo pipefail\n{interpolated}");
    let status = std::process::Command::new("bash")
        .arg("-c")
        .arg(&wrapped)
        .current_dir(project_root)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        std::process::exit(code);
    }

    Ok(())
}

async fn dispatch_job(
    client: &DaemonClient,
    namespace: &str,
    command: &str,
    job_id: &str,
    job_name: &str,
) -> Result<()> {
    let short_id = job_id.short(8);
    println!("{} {namespace}", color::context("Project:"));
    println!("{} {command}", color::context("Command:"));
    println!(
        "{} {}",
        color::yellow("Waiting for job to start..."),
        color::muted("(Ctrl+C to skip)")
    );
    println!();

    // Poll for job start
    let mut poller = crate::poll::Poller::new(
        Duration::from_millis(500),
        Some(crate::client::run_wait_timeout()),
    );
    let mut started = false;

    loop {
        match poller.tick().await {
            crate::poll::Tick::Ready => {}
            _ => break,
        }
        if let Ok(Some(p)) = client.get_job(job_id).await {
            if p.step_status != "pending" {
                started = true;
                break;
            }
        }
    }

    if started {
        println!(
            "{} {} {}",
            color::green("Started"),
            job_name,
            color::muted(&format!("(job: {short_id})"))
        );
    } else {
        println!(
            "{}",
            color::yellow("Still waiting for the job to start, check:")
        );
    }
    super::job::print_job_commands(short_id);

    Ok(())
}

async fn dispatch_agent_run(
    client: &DaemonClient,
    namespace: &str,
    command: &str,
    agent_run_id: &str,
    agent_name: &str,
    should_attach: bool,
) -> Result<()> {
    let short_id = agent_run_id.short(8);
    println!("{} {namespace}", color::context("Project:"));
    println!("{} {command}", color::context("Command:"));
    println!(
        "{} {agent_name} {}",
        color::context("Agent:"),
        color::muted(&format!("({short_id})"))
    );
    println!();

    if !should_attach || !std::io::stdout().is_terminal() {
        println!("  oj agent show {short_id}");
        println!("  oj agent logs {short_id}");
        return Ok(());
    }

    // Poll for session_id to appear on the agent run
    println!(
        "{} {}",
        color::yellow("Waiting for agent session..."),
        color::muted("(Ctrl+C to skip)")
    );

    let mut poller =
        crate::poll::Poller::new(Duration::from_millis(300), Some(Duration::from_secs(15)));
    let mut session_id = None;

    loop {
        match poller.tick().await {
            crate::poll::Tick::Ready => {}
            _ => break,
        }
        if let Ok(Some(detail)) = client.get_agent(agent_run_id).await {
            if let Some(ref sid) = detail.session_id {
                session_id = Some(sid.clone());
                break;
            }
        }
    }

    match session_id {
        Some(sid) => {
            crate::commands::session::attach(&sid)?;
        }
        None => {
            println!(
                "{}",
                color::yellow("Agent session not ready yet. You can attach manually:")
            );
            println!("  oj attach {short_id}");
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
