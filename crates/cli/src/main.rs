// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! oj - Odd Jobs CLI

mod client;
mod client_lifecycle;
mod color;
mod commands;
mod daemon_process;
mod env;
mod exit_error;
mod help;
mod output;
mod table;

use output::OutputFormat;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use commands::{
    agent, cron, daemon, decision, emit, env as env_cmd, job, project, queue, resolve, run,
    runbook, session, status, worker, workspace,
};
use std::path::{Path, PathBuf};

use crate::client::DaemonClient;

#[derive(Parser)]
#[command(
    name = "oj",
    version,
    disable_version_flag = true,
    about = "Odd Jobs - An automated team for your odd jobs"
)]
struct Cli {
    /// Change to <dir> before doing anything
    #[arg(short = 'C', global = true, value_name = "DIR")]
    directory: Option<PathBuf>,

    /// Project namespace override
    #[arg(long = "project", global = true)]
    project: Option<String>,

    /// Output format
    #[arg(
        short = 'o',
        long = "output",
        value_enum,
        default_value_t,
        global = true
    )]
    output: OutputFormat,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a command from the runbook
    Run(run::RunArgs),
    /// Job management
    Job(job::JobArgs),
    /// Agent management
    Agent(agent::AgentArgs),
    /// Session management
    Session(session::SessionArgs),
    /// Workspace management
    Workspace(workspace::WorkspaceArgs),
    /// Daemon management
    Daemon(daemon::DaemonArgs),
    /// Environment variable management
    Env(env_cmd::EnvArgs),
    /// Queue management
    Queue(queue::QueueArgs),
    /// Worker management
    Worker(worker::WorkerArgs),
    /// Cron management
    Cron(cron::CronArgs),
    /// Decision management
    Decision(decision::DecisionArgs),
    /// Emit events to the daemon (for agents)
    Emit(emit::EmitArgs),
    /// Project management
    Project(project::ProjectArgs),
    /// Runbook management
    Runbook(runbook::RunbookArgs),
    /// Show overview of active work across all projects
    Status(status::StatusArgs),
    /// Peek at the active tmux session (auto-detects entity type)
    Peek {
        /// Entity ID (job, agent, or session — prefix match supported)
        id: String,
    },
    /// Attach to a tmux session (auto-detects entity type)
    Attach {
        /// Entity ID (job, agent, or session — prefix match supported)
        id: String,
    },
    /// View logs for a job or agent (auto-detects entity type)
    Logs {
        /// Entity ID (job or agent — prefix match supported)
        id: String,
        /// Stream live activity (like tail -f)
        #[arg(long, short)]
        follow: bool,
        /// Number of recent lines to show (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
        /// Show only a specific step's log (agent logs only)
        #[arg(long, short = 's')]
        step: Option<String>,
    },
    /// Show details of a job, agent, session, or queue (auto-detects type)
    Show {
        /// Entity ID or queue name (job, agent, session, or queue)
        id: String,
        /// Show full variable values without truncation
        #[arg(long, short = 'v')]
        verbose: bool,
    },
    /// Cancel one or more running jobs
    Cancel {
        /// Job IDs or names (prefix match)
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Resume monitoring for an escalated job
    Resume {
        /// Job ID or name. Required unless --all is used.
        id: Option<String>,
        /// Message for nudge/recovery (required for agent steps)
        #[arg(short = 'm', long)]
        message: Option<String>,
        /// Job variables to set (can be repeated: --var key=value)
        #[arg(long = "var", value_parser = job::parse_key_value)]
        var: Vec<(String, String)>,
        /// Kill running agent and restart (still preserves conversation via --resume)
        #[arg(long)]
        kill: bool,
        /// Resume all resumable jobs (waiting/failed/pending)
        #[arg(long)]
        all: bool,
    },
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        let code = e
            .downcast_ref::<exit_error::ExitError>()
            .map_or(1, |c| c.code);
        let msg = format_error(&e);
        if !msg.is_empty() {
            eprintln!("Error: {}", msg);
        }
        std::process::exit(code);
    }
}

/// Format an anyhow error, deduplicating the chain.
///
/// If the top-level Display already contains the source error text, we skip
/// the "Caused by" chain to avoid noisy duplicate output (common when
/// thiserror variants use `#[error("... {0}")]` with `#[from]`).
/// Otherwise we render the full chain so context isn't lost.
fn format_error(err: &anyhow::Error) -> String {
    let top = err.to_string();

    // Walk the source chain; if every source message already appears
    // in the top-level string, the chain is redundant.
    let chain_redundant = err
        .chain()
        .skip(1)
        .all(|cause| top.contains(&cause.to_string()));

    if chain_redundant {
        return top;
    }

    // Non-redundant chain — render like anyhow's Debug.
    let mut buf = top;
    for (i, cause) in err.chain().skip(1).enumerate() {
        buf.push_str(&format!("\n\nCaused by:\n    {}: {}", i, cause));
    }
    buf
}

pub(crate) fn cli_command() -> clap::Command {
    // Check for -C in raw args to discover correct project root for help text
    let project_root = find_project_root_from_args();
    let run_help = commands::run::available_commands_help(&project_root);

    Cli::command()
        .help_template(help::template())
        .before_help(help::commands())
        .after_help(help::after_help())
        .styles(help::styles())
        .arg(
            clap::Arg::new("version")
                .short('v')
                .short_alias('V')
                .long("version")
                .action(clap::ArgAction::Version)
                .help("Print version"),
        )
        .mut_subcommand("daemon", |sub| {
            sub.mut_arg("version", |arg| arg.short_alias('V'))
        })
        .mut_subcommand("run", |sub| sub.override_help(run_help))
}

async fn run() -> Result<()> {
    let matches = match cli_command().try_get_matches() {
        Ok(m) => m,
        Err(e) => {
            match e.kind() {
                clap::error::ErrorKind::DisplayHelp => {
                    // Intercept help requests → post-hoc colorized output
                    let args: Vec<String> = std::env::args().collect();
                    let args = strip_global_flags(&args);
                    print_formatted_help(&args);
                    return Ok(());
                }
                clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    // Missing required subcommand (e.g. `oj project`) — colorize
                    let args: Vec<String> = std::env::args().collect();
                    let args = strip_global_flags(&args);
                    print_formatted_help(&args);
                    return Err(exit_error::ExitError::new(2, "").into());
                }
                _ => {
                    // DisplayVersion and other errors: let clap handle
                    e.exit();
                }
            }
        }
    };
    let cli = Cli::from_arg_matches(&matches)?;
    let format = cli.output;

    // Apply -C: change working directory early, before project root discovery
    if let Some(ref dir) = cli.directory {
        let canonical = std::fs::canonicalize(dir).map_err(|e| {
            anyhow::anyhow!("cannot change to directory '{}': {}", dir.display(), e)
        })?;
        std::env::set_current_dir(&canonical).map_err(|e| {
            anyhow::anyhow!(
                "cannot change to directory '{}': {}",
                canonical.display(),
                e
            )
        })?;
    }

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            // No subcommand provided — print colorized help and exit 0
            help::print_help(cli_command());
            return Ok(());
        }
    };

    // Handle daemon command separately (doesn't need client connection)
    if let Commands::Daemon(args) = command {
        return daemon::daemon(args, format).await;
    }

    // Handle env command separately (doesn't need client connection)
    if let Commands::Env(args) = command {
        return env_cmd::handle(args.command, format);
    }

    // Handle runbook command separately (doesn't need client connection)
    if let Commands::Runbook(args) = command {
        let project_root = find_project_root();
        return runbook::handle(args.command, &project_root, format);
    }

    // Find project root for runbook loading (now from potentially-changed cwd)
    let project_root = find_project_root();
    let invoke_dir = std::env::current_dir().unwrap_or_else(|_| project_root.clone());

    // Centralized namespace resolution:
    //   --project flag > OJ_NAMESPACE env > auto-resolved from project root
    let namespace = resolve_effective_namespace(cli.project.as_deref(), &project_root);

    // Explicit --project flag for filtering list/show queries.
    // OJ_NAMESPACE is NOT used for filtering — only the explicit CLI flag.
    let project_filter = cli.project.as_deref();

    // Dispatch commands with appropriate client semantics.
    // Each command enum provides client_kind() to determine action/query/signal.
    match command {
        Commands::Run(args) => run::handle(args, &project_root, &invoke_dir, &namespace).await?,

        Commands::Job(args) => {
            let client = DaemonClient::for_kind(args.command.client_kind())?;
            job::handle(args.command, &client, cli.project.as_deref(), format).await?
        }
        Commands::Workspace(args) => {
            let client = DaemonClient::for_kind(args.command.client_kind())?;
            workspace::handle(args.command, &client, &namespace, project_filter, format).await?
        }
        Commands::Agent(args) => {
            let client = DaemonClient::for_kind(args.command.client_kind())?;
            agent::handle(args.command, &client, &namespace, project_filter, format).await?
        }
        Commands::Session(args) => {
            let client = DaemonClient::for_kind(args.command.client_kind())?;
            session::handle(args.command, &client, &namespace, project_filter, format).await?
        }
        Commands::Queue(args) => {
            let client = DaemonClient::for_kind(args.command.client_kind())?;
            queue::handle(args.command, &client, &project_root, &namespace, format).await?
        }
        Commands::Worker(args) => {
            let client = DaemonClient::for_kind(args.command.client_kind())?;
            worker::handle(
                args.command,
                &client,
                &project_root,
                &namespace,
                project_filter,
                format,
            )
            .await?
        }
        Commands::Cron(args) => {
            let client = DaemonClient::for_kind(args.command.client_kind())?;
            cron::handle(
                args.command,
                &client,
                &project_root,
                &namespace,
                project_filter,
                format,
            )
            .await?
        }
        Commands::Decision(args) => {
            let client = DaemonClient::for_kind(args.command.client_kind())?;
            decision::handle(args.command, &client, &namespace, project_filter, format).await?
        }
        Commands::Emit(args) => {
            let client = DaemonClient::for_signal()?;
            emit::handle(args.command, &client, format).await?
        }

        // Project - global cross-project listing (query, graceful when daemon down)
        Commands::Project(args) => {
            project::handle_not_running_or(args.command, format).await?;
        }

        // Status - top-level dashboard (query, graceful when daemon down)
        Commands::Status(args) => {
            status::handle(args, format, project_filter).await?;
        }

        // Convenience commands - resolve entity type automatically (query)
        Commands::Peek { id } => {
            let client = DaemonClient::for_query()?;
            resolve::handle_peek(&client, &id, format).await?
        }
        Commands::Attach { id } => {
            let client = DaemonClient::for_query()?;
            resolve::handle_attach(&client, &id).await?
        }
        Commands::Logs {
            id,
            follow,
            limit,
            step,
        } => {
            let client = DaemonClient::for_query()?;
            resolve::handle_logs(&client, &id, follow, limit, step.as_deref(), format).await?
        }
        Commands::Show { id, verbose } => {
            let client = DaemonClient::for_query()?;
            let matches = resolve::resolve_entity(&client, &id).await?;
            if matches.is_empty() {
                // No entity match — try as a queue name
                queue::handle(
                    queue::QueueCommand::Show { queue: id },
                    &client,
                    &project_root,
                    &namespace,
                    format,
                )
                .await?
            } else {
                resolve::handle_show(&client, &id, verbose, format).await?
            }
        }

        // Convenience action commands - cancel/resume jobs
        Commands::Cancel { ids } => {
            let client = DaemonClient::for_action()?;
            job::handle(
                job::JobCommand::Cancel { ids },
                &client,
                cli.project.as_deref(),
                format,
            )
            .await?
        }
        Commands::Resume {
            id,
            message,
            var,
            kill,
            all,
        } => {
            let client = DaemonClient::for_action()?;
            job::handle(
                job::JobCommand::Resume {
                    id,
                    message,
                    var,
                    kill,
                    all,
                },
                &client,
                cli.project.as_deref(),
                format,
            )
            .await?
        }

        Commands::Daemon(_) | Commands::Env(_) | Commands::Runbook(_) => unreachable!(),
    }

    Ok(())
}

pub fn load_runbook(
    project_root: &std::path::Path,
    name: &str,
    runbook_file: Option<&str>,
) -> Result<oj_runbook::Runbook> {
    let runbook_dir = project_root.join(".oj/runbooks");

    // If --runbook flag provided, load only that file
    if let Some(file) = runbook_file {
        let path = runbook_dir.join(file);
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("failed to read runbook '{}': {}", file, e))?;
        let format = match path.extension().and_then(|e| e.to_str()) {
            Some("hcl") => oj_runbook::Format::Hcl,
            Some("json") => oj_runbook::Format::Json,
            _ => oj_runbook::Format::Toml,
        };
        return Ok(oj_runbook::parse_runbook_with_format(&content, format)?);
    }

    let runbook = oj_runbook::find_runbook_by_command(&runbook_dir, name)?;
    runbook.ok_or_else(|| anyhow::anyhow!("unknown command: {}", name))
}

/// Find project root, honoring a -C flag if present in raw argv.
/// Used by `cli_command()` for help text generation before full argument parsing.
fn find_project_root_from_args() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "-C" {
            if let Some(dir) = args.get(i + 1) {
                if let Ok(canonical) = std::fs::canonicalize(dir) {
                    return find_project_root_from(canonical);
                }
            }
        }
    }
    find_project_root()
}

/// Resolve the effective namespace using the standard priority chain:
///   --project flag > OJ_NAMESPACE env > project root resolution
fn resolve_effective_namespace(project: Option<&str>, project_root: &Path) -> String {
    if let Some(p) = project {
        return p.to_string();
    }
    if let Some(ns) = crate::env::namespace() {
        return ns;
    }
    oj_core::namespace::resolve_namespace(project_root)
}

/// Find the project root by walking up from current directory.
/// Looks for .oj directory to identify project root.
///
/// When running inside a git worktree (e.g. a workspace),
/// resolves to the main worktree's project root so that daemon requests
/// (queue push, worker start, etc.) reference the canonical project.
fn find_project_root() -> PathBuf {
    let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    find_project_root_from(start)
}

/// Find the project root by walking up from a given starting directory.
fn find_project_root_from(start: PathBuf) -> PathBuf {
    let mut current = start;
    loop {
        if current.join(".oj").is_dir() {
            return resolve_main_worktree(&current).unwrap_or(current);
        }
        if !current.pop() {
            // No .oj directory found, use current directory
            return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        }
    }
}

/// If `path` is inside a git worktree, resolve to the main worktree root.
/// Returns None if `path` is already the main worktree (or not a git repo).
fn resolve_main_worktree(path: &Path) -> Option<PathBuf> {
    let git_path = path.join(".git");

    // In a worktree, .git is a file containing "gitdir: <path>"
    // In the main worktree, .git is a directory
    if !git_path.is_file() {
        return None;
    }

    let content = std::fs::read_to_string(&git_path).ok()?;
    let gitdir = content.strip_prefix("gitdir: ")?.trim();

    let gitdir_path = if Path::new(gitdir).is_absolute() {
        PathBuf::from(gitdir)
    } else {
        path.join(gitdir)
    };

    // gitdir points to <main>/.git/worktrees/<name>
    // Walk up: worktrees/ -> .git/ -> main project root
    let main_git_dir = gitdir_path.parent()?.parent()?;
    let main_root = main_git_dir.parent()?;

    if main_root.join(".oj").is_dir() {
        Some(main_root.to_path_buf())
    } else {
        None
    }
}

/// Print help with post-hoc colorization, resolving the correct subcommand from args.
fn print_formatted_help(args: &[String]) {
    let mut cmd = cli_command();
    // Propagate global args and bin_name to subcommands so that
    // cloned subcommands include -C, --project, -o and show the
    // correct "oj <subcommand>" usage prefix.
    cmd.build();

    // Extract subcommand names from args (skip binary name and flags).
    // Handle both "oj run --help" and "oj help run" patterns.
    let non_flags: Vec<&String> = args
        .iter()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .collect();

    let subcommand_names: Vec<&str> = if non_flags.first().map(|s| s.as_str()) == Some("help") {
        non_flags.iter().skip(1).map(|s| s.as_str()).collect()
    } else {
        non_flags.iter().map(|s| s.as_str()).collect()
    };

    // Route "run <command>" to per-command help
    if subcommand_names.first() == Some(&"run") && subcommand_names.len() > 1 {
        let command_name = subcommand_names[1];
        let project_root = find_project_root_from_args();
        let runbook_dir = project_root.join(".oj/runbooks");

        if let Ok(Some(runbook)) = oj_runbook::find_runbook_by_command(&runbook_dir, command_name) {
            if let Some(cmd_def) = runbook.get_command(command_name) {
                let comment = oj_runbook::find_command_with_comment(&runbook_dir, command_name)
                    .ok()
                    .flatten()
                    .and_then(|(_, comment)| comment);

                help::print_help_text(&cmd_def.format_help(command_name, comment.as_ref()));
                return;
            }
        }
        // Fall through to normal help if command not found
    }

    let target_cmd = find_subcommand(cmd, &subcommand_names);
    help::print_help(target_cmd);
}

/// Strip `-C <value>` and `--project <value>` from args to avoid mistaking
/// their values for subcommand names in help formatting.
fn strip_global_flags(args: &[String]) -> Vec<String> {
    let mut result = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "-C" || arg == "--project" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("-C") && arg.len() > 2 {
            continue;
        }
        if arg.starts_with("--project=") {
            continue;
        }
        result.push(arg.clone());
    }
    result
}

/// Recursively find a nested subcommand by name path.
pub(crate) fn find_subcommand(mut cmd: clap::Command, names: &[&str]) -> clap::Command {
    for name in names {
        let mut found_sub = None;
        for sub in cmd.get_subcommands() {
            if sub.get_name() == *name || sub.get_all_aliases().any(|a| a == *name) {
                found_sub = Some(sub.get_name().to_string());
                break;
            }
        }
        if let Some(sub_name) = found_sub {
            if let Some(sub) = cmd.find_subcommand_mut(&sub_name) {
                cmd = sub.clone();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    cmd
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
