// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj session` - Session management commands

use std::io::Write;

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::color;
use crate::output::{format_time_ago, should_use_color, OutputFormat};
use oj_daemon::protocol::SessionSummary;

#[derive(Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: SessionCommand,
}

#[derive(Subcommand)]
pub enum SessionCommand {
    /// List all sessions
    List {
        /// Filter by project namespace
        #[arg(long = "project")]
        project: Option<String>,
    },
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
    format: OutputFormat,
) -> Result<()> {
    match command {
        SessionCommand::List { project } => {
            let mut sessions = client.list_sessions().await?;

            // Filter by project namespace
            let filter_namespace = project.or_else(|| std::env::var("OJ_NAMESPACE").ok());
            if let Some(ref ns) = filter_namespace {
                sessions.retain(|s| s.namespace == *ns);
            }

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
    // Determine whether to show PROJECT column
    let namespaces: std::collections::HashSet<&str> =
        sessions.iter().map(|s| s.namespace.as_str()).collect();
    let show_project = namespaces.len() > 1 || namespaces.iter().any(|n| !n.is_empty());

    // Calculate column widths based on data
    let session_w = sessions
        .iter()
        .map(|s| s.id.len())
        .max()
        .unwrap_or(0)
        .max("SESSION".len());
    let no_project = "(no project)";
    let proj_w = if show_project {
        sessions
            .iter()
            .map(|s| {
                if s.namespace.is_empty() {
                    no_project.len()
                } else {
                    s.namespace.len()
                }
            })
            .max()
            .unwrap_or(7)
            .max(7)
    } else {
        0
    };
    let pipeline_w = sessions
        .iter()
        .map(|s| s.pipeline_id.as_ref().map(|p| p.len()).unwrap_or(1))
        .max()
        .unwrap_or(0)
        .max("PIPELINE".len());

    if show_project {
        let _ = writeln!(
            w,
            "{} {} {} {}",
            color::header(&format!("{:<session_w$}", "SESSION")),
            color::header(&format!("{:<proj_w$}", "PROJECT")),
            color::header(&format!("{:<pipeline_w$}", "PIPELINE")),
            color::header("UPDATED"),
        );
    } else {
        let _ = writeln!(
            w,
            "{} {} {}",
            color::header(&format!("{:<session_w$}", "SESSION")),
            color::header(&format!("{:<pipeline_w$}", "PIPELINE")),
            color::header("UPDATED"),
        );
    }
    for s in sessions {
        let updated_ago = format_time_ago(s.updated_at_ms);
        let pipeline = s.pipeline_id.as_deref().unwrap_or("-");
        if show_project {
            let proj = if s.namespace.is_empty() {
                no_project
            } else {
                &s.namespace
            };
            let _ = writeln!(
                w,
                "{} {:<proj_w$} {:<pipeline_w$} {}",
                color::muted(&format!("{:<session_w$}", &s.id)),
                proj,
                pipeline,
                updated_ago
            );
        } else {
            let _ = writeln!(
                w,
                "{} {:<pipeline_w$} {}",
                color::muted(&format!("{:<session_w$}", &s.id)),
                pipeline,
                updated_ago
            );
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
