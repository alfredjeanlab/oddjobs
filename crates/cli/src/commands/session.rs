// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj session` - Session management commands

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::output::{format_time_ago, should_use_color, OutputFormat};

#[derive(Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: SessionCommand,
}

#[derive(Subcommand)]
pub enum SessionCommand {
    /// List all sessions
    List,
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
    format: OutputFormat,
) -> Result<()> {
    match command {
        SessionCommand::List => {
            let sessions = client.list_sessions().await?;

            match format {
                OutputFormat::Text => {
                    if sessions.is_empty() {
                        println!("No sessions");
                    } else {
                        // Calculate column widths based on data
                        let session_width = sessions
                            .iter()
                            .map(|s| s.id.len())
                            .max()
                            .unwrap_or(0)
                            .max("SESSION".len());
                        let pipeline_width = sessions
                            .iter()
                            .map(|s| {
                                s.pipeline_id.as_ref().map(|p| p.len()).unwrap_or(1)
                                // "-" is 1 char
                            })
                            .max()
                            .unwrap_or(0)
                            .max("PIPELINE".len());

                        println!(
                            "{:<session_width$} {:<pipeline_width$} UPDATED",
                            "SESSION", "PIPELINE"
                        );
                        for s in sessions {
                            let updated_ago = format_time_ago(s.updated_at_ms);
                            println!(
                                "{:<session_width$} {:<pipeline_width$} {}",
                                s.id,
                                s.pipeline_id.as_deref().unwrap_or("-"),
                                updated_ago
                            );
                        }
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
        SessionCommand::Attach { id } => {
            attach(&id)?;
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
