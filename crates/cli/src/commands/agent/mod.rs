// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent management commands

pub(crate) mod display;
mod hooks;
mod utils;
mod wait;

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::{ClientKind, DaemonClient};
use crate::output::OutputFormat;

#[derive(Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommand,
}

#[derive(Subcommand)]
pub enum AgentCommand {
    /// List agents across all jobs
    List {
        /// Filter by job ID (or prefix)
        #[arg(long)]
        job: Option<String>,

        /// Filter by status (e.g. "running", "completed", "failed", "waiting")
        #[arg(long)]
        status: Option<String>,

        /// Maximum number of agents to show (default: 20)
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,

        /// Show all agents (no limit)
        #[arg(long, conflicts_with = "limit")]
        no_limit: bool,
    },
    /// Show detailed info for a single agent
    Show {
        /// Agent ID (or prefix)
        id: String,
    },
    /// Send a message to a running agent
    Send {
        /// Agent ID or job ID (or prefix)
        agent_id: String,
        /// Message to send
        message: String,
    },
    /// View agent activity log
    Logs {
        /// Agent ID or job ID (or prefix)
        id: String,
        /// Show only a specific step's log
        #[arg(long, short = 's')]
        step: Option<String>,
        /// Stream live activity (like tail -f)
        #[arg(long, short)]
        follow: bool,
        /// Number of recent lines to show per step (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
    },
    /// Block until a specific agent reaches a terminal or idle state
    Wait {
        /// Agent ID (or prefix)
        agent_id: String,
        /// Timeout duration (e.g. "5m", "30s", "1h")
        #[arg(long)]
        timeout: Option<String>,
    },
    /// Peek at an agent's tmux session output
    Peek {
        /// Agent ID (or prefix)
        id: String,
    },
    /// Attach to an agent's tmux session
    Attach {
        /// Agent ID (or prefix)
        id: String,
    },
    /// Kill an agent's session (triggers on_dead lifecycle)
    Kill {
        /// Agent ID or job ID (or prefix)
        id: String,
    },
    /// Remove agent logs from completed/failed/cancelled jobs
    Prune {
        /// Remove all agent logs from terminal jobs regardless of age
        #[arg(long)]
        all: bool,
        /// Show what would be pruned without doing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Resume a dead agent's session (re-spawn with --resume to preserve conversation)
    Resume {
        /// Agent ID (or prefix). Required unless --all is used.
        id: Option<String>,
        /// Force kill the current tmux session before resuming
        #[arg(long)]
        kill: bool,
        /// Resume all agents that have dead sessions
        #[arg(long)]
        all: bool,
    },
    /// Hook subcommands for Claude Code integration
    Hook {
        #[command(subcommand)]
        hook: HookCommand,
    },
}

impl AgentCommand {
    pub fn client_kind(&self) -> ClientKind {
        match self {
            Self::Send { .. } | Self::Kill { .. } | Self::Resume { .. } | Self::Prune { .. } => {
                ClientKind::Action
            }
            Self::Hook { .. } => ClientKind::Signal,
            _ => ClientKind::Query,
        }
    }
}

#[derive(Subcommand)]
pub enum HookCommand {
    /// Stop hook handler - gates agent completion
    Stop {
        /// Agent ID to check
        agent_id: String,
    },
    /// PreToolUse hook handler - detects plan/question tools and transitions to Prompting
    Pretooluse {
        /// Agent ID to emit prompt event for
        agent_id: String,
    },
    /// Notification hook handler - detects idle_prompt and permission_prompt
    Notify {
        /// Agent ID to emit state events for
        #[arg(long)]
        agent_id: String,
    },
}

pub async fn handle(
    command: AgentCommand,
    client: &DaemonClient,
    _namespace: &str,
    project_filter: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        AgentCommand::List {
            job,
            status,
            limit,
            no_limit,
        } => {
            display::handle_list(client, project_filter, format, job, status, limit, no_limit)
                .await?;
        }
        AgentCommand::Show { id } => {
            display::handle_show(client, format, &id).await?;
        }
        AgentCommand::Peek { id } => {
            display::handle_peek(client, &id).await?;
        }
        AgentCommand::Attach { id } => {
            display::handle_attach(client, &id).await?;
        }
        AgentCommand::Kill { id } => {
            display::handle_kill(client, &id).await?;
        }
        AgentCommand::Send { agent_id, message } => {
            display::handle_send(client, &agent_id, &message).await?;
        }
        AgentCommand::Logs {
            id,
            step,
            follow,
            limit,
        } => {
            display::handle_logs(client, format, &id, step.as_deref(), follow, limit).await?;
        }
        AgentCommand::Wait { agent_id, timeout } => {
            wait::handle_wait(&agent_id, timeout.as_deref(), client).await?;
        }
        AgentCommand::Prune { all, dry_run } => {
            display::handle_prune(client, format, all, dry_run).await?;
        }
        AgentCommand::Resume { id, kill, all } => {
            display::handle_resume(client, format, id, kill, all).await?;
        }
        AgentCommand::Hook { hook } => match hook {
            HookCommand::Stop { agent_id } => {
                hooks::handle_stop(&agent_id, client).await?;
            }
            HookCommand::Pretooluse { agent_id } => {
                hooks::handle_pretooluse(&agent_id, client).await?;
            }
            HookCommand::Notify { agent_id } => {
                hooks::handle_notify(&agent_id, client).await?;
            }
        },
    }

    Ok(())
}
