// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj session` - Session management commands

use std::io::Write;

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::output::{format_time_ago, should_use_color, OutputFormat};
use crate::table::{project_cell, should_show_project, Column, Table};
use oj_daemon::protocol::SessionSummary;

#[derive(Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: SessionCommand,
}

#[derive(Subcommand)]
pub enum SessionCommand {
    /// List all sessions
    List {},
    /// Send input to a session
    Send {
        /// Session ID
        id: String,
        /// Input to send
        input: String,
    },
    /// Peek at a tmux session's terminal output
    Peek {
        /// Session ID
        id: String,
    },
    /// Kill a session
    Kill {
        /// Session ID
        id: String,
    },
    /// Attach to a session (opens tmux)
    Attach {
        /// Session ID
        id: String,
    },
}

/// Attach to a tmux session
pub fn attach(id: &str) -> Result<()> {
    let status = std::process::Command::new("tmux")
        .args(["attach", "-t", id])
        .status()?;

    if !status.success() {
        anyhow::bail!("Failed to attach to session {}", id);
    }
    Ok(())
}

pub async fn handle(
    command: SessionCommand,
    client: &DaemonClient,
    _namespace: &str,
    format: OutputFormat,
) -> Result<()> {
    match command {
        SessionCommand::List {} => {
            let sessions = client.list_sessions().await?;

            match format {
                OutputFormat::Text => {
                    if sessions.is_empty() {
                        println!("No sessions");
                    } else {
                        format_session_list(&mut std::io::stdout(), &sessions);
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&sessions)?);
                }
            }
        }
        SessionCommand::Peek { id } => {
            let with_color = should_use_color();
            match client.peek_session(&id, with_color).await {
                Ok(output) => {
                    println!("╭──── peek: {} ────", id);
                    print!("{}", output);
                    println!("╰──── end peek ────");
                }
                Err(_) => {
                    anyhow::bail!("Session {} not found", id);
                }
            }
        }
        SessionCommand::Send { id, input } => {
            client.session_send(&id, &input).await?;
            println!("Sent to session {}", id);
        }
        SessionCommand::Kill { id } => {
            client.session_kill(&id).await?;
            println!("Killed session {}", id);
        }
        SessionCommand::Attach { id } => {
            attach(&id)?;
        }
    }

    Ok(())
}

fn format_session_list(w: &mut impl Write, sessions: &[SessionSummary]) {
    let show_project = should_show_project(sessions.iter().map(|s| s.namespace.as_str()));

    let mut cols = vec![Column::muted("SESSION")];
    if show_project {
        cols.push(Column::left("PROJECT"));
    }
    cols.extend([Column::left("PIPELINE"), Column::left("UPDATED")]);
    let mut table = Table::new(cols);

    for s in sessions {
        let pipeline = s.pipeline_id.as_deref().unwrap_or("-").to_string();
        let updated = format_time_ago(s.updated_at_ms);
        let mut cells = vec![s.id.clone()];
        if show_project {
            cells.push(project_cell(&s.namespace));
        }
        cells.extend([pipeline, updated]);
        table.row(cells);
    }

    table.render(w);
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
