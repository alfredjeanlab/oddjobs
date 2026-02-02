// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! oj - Odd Jobs CLI

mod client;
mod commands;
mod daemon_process;
mod exit_error;
mod output;

use output::OutputFormat;

use anyhow::Result;
use clap::{Parser, Subcommand};
use commands::{agent, daemon, emit, pipeline, queue, run, session, worker, workspace};
use std::path::{Path, PathBuf};

use crate::client::DaemonClient;

#[derive(Parser)]
#[command(
    name = "oj",
    version,
    about = "Odd Jobs - Agentic development automation"
)]
struct Cli {
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
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a command from the runbook
    Run(run::RunArgs),
    /// Pipeline management
    Pipeline(pipeline::PipelineArgs),
    /// Agent management
    Agent(agent::AgentArgs),
    /// Session management
    Session(session::SessionArgs),
    /// Workspace management
    Workspace(workspace::WorkspaceArgs),
    /// Daemon management
    Daemon(daemon::DaemonArgs),
    /// Queue management
    Queue(queue::QueueArgs),
    /// Worker management
    Worker(worker::WorkerArgs),
    /// Emit events to the daemon (for agents)
    Emit(emit::EmitArgs),
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        let code = e
            .downcast_ref::<exit_error::ExitError>()
            .map_or(1, |c| c.code);
        eprintln!("Error: {}", format_error(&e));
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

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let format = cli.output;

    // Handle daemon command separately (doesn't need client connection)
    if let Commands::Daemon(args) = cli.command {
        return daemon::daemon(args, format).await;
    }

    // Find project root for runbook loading
    let project_root = find_project_root();
    let invoke_dir = std::env::current_dir().unwrap_or_else(|_| project_root.clone());
    let namespace = oj_core::namespace::resolve_namespace(&project_root);

    // Dispatch commands with appropriate client semantics:
    // - Action commands: auto-start daemon, max 1 restart (user-initiated mutations)
    // - Query commands: connect only, no restart (reads that need existing state)
    // - Signal commands: connect only, no restart (agent-initiated, context-dependent)
    match cli.command {
        // Action commands - mutate state, user-initiated
        Commands::Run(args) => {
            let client = DaemonClient::for_action()?;
            run::handle(args, &client, &project_root, &invoke_dir, &namespace).await?
        }

        // Pipeline commands - mixed action/query
        Commands::Pipeline(args) => {
            use pipeline::PipelineCommand;
            match &args.command {
                // Action: mutates pipeline state
                PipelineCommand::Resume { .. } | PipelineCommand::Cancel { .. } => {
                    let client = DaemonClient::for_action()?;
                    pipeline::handle(args.command, &client, format).await?
                }
                // Query: reads pipeline state
                PipelineCommand::List { .. }
                | PipelineCommand::Show { .. }
                | PipelineCommand::Logs { .. }
                | PipelineCommand::Peek { .. }
                | PipelineCommand::Wait { .. }
                | PipelineCommand::Attach { .. } => {
                    let client = DaemonClient::for_query()?;
                    pipeline::handle(args.command, &client, format).await?
                }
            }
        }

        // Workspace commands - mixed action/query
        Commands::Workspace(args) => {
            use workspace::WorkspaceCommand;
            match &args.command {
                // Action: mutates workspace state
                WorkspaceCommand::Drop { .. } | WorkspaceCommand::Prune { .. } => {
                    let client = DaemonClient::for_action()?;
                    workspace::handle(args.command, &client, format).await?
                }
                // Query: reads workspace state
                WorkspaceCommand::List { .. } | WorkspaceCommand::Show { .. } => {
                    let client = DaemonClient::for_query()?;
                    workspace::handle(args.command, &client, format).await?
                }
            }
        }

        // Agent commands - mixed action/query
        Commands::Agent(args) => {
            use agent::AgentCommand;
            match &args.command {
                // Action: sends input to an agent
                AgentCommand::Send { .. } => {
                    let client = DaemonClient::for_action()?;
                    agent::handle(args.command, &client, format).await?
                }
                // Query: reads agent state
                _ => {
                    let client = DaemonClient::for_query()?;
                    agent::handle(args.command, &client, format).await?
                }
            }
        }
        Commands::Session(args) => {
            let client = DaemonClient::for_query()?;
            session::handle(args.command, &client, format).await?
        }

        // Queue commands - mixed action/query
        Commands::Queue(args) => {
            use queue::QueueCommand;
            match &args.command {
                QueueCommand::Push { .. }
                | QueueCommand::Drop { .. }
                | QueueCommand::Retry { .. } => {
                    let client = DaemonClient::for_action()?;
                    queue::handle(args.command, &client, &project_root, &namespace, format).await?
                }
                QueueCommand::List { .. } => {
                    let client = DaemonClient::for_query()?;
                    queue::handle(args.command, &client, &project_root, &namespace, format).await?
                }
            }
        }

        // Worker commands - mixed action/query
        Commands::Worker(args) => match &args.command {
            worker::WorkerCommand::List { .. } => {
                let client = DaemonClient::for_query()?;
                worker::handle(args.command, &client, &project_root, &namespace, format).await?
            }
            _ => {
                let client = DaemonClient::for_action()?;
                worker::handle(args.command, &client, &project_root, &namespace, format).await?
            }
        },

        // Signal commands - operational, agent-initiated
        Commands::Emit(args) => {
            let client = DaemonClient::for_signal()?;
            emit::handle(args.command, &client, format).await?
        }

        Commands::Daemon(_) => unreachable!(),
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

/// Find the project root by walking up from current directory.
/// Looks for .oj directory to identify project root.
///
/// When running inside a git worktree (e.g. an ephemeral workspace),
/// resolves to the main worktree's project root so that daemon requests
/// (queue push, worker start, etc.) reference the canonical project.
fn find_project_root() -> PathBuf {
    let Ok(mut current) = std::env::current_dir() else {
        return PathBuf::from(".");
    };

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
