// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj project` — project management commands.

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::color;
use crate::output::OutputFormat;

#[derive(Args)]
pub struct ProjectArgs {
    #[command(subcommand)]
    pub command: ProjectCommand,
}

#[derive(Subcommand)]
pub enum ProjectCommand {
    /// List projects with active work
    List {},
}

/// Entry point that handles daemon-not-running gracefully (like `oj status`).
pub async fn handle_not_running_or(command: ProjectCommand, format: OutputFormat) -> Result<()> {
    let client = match DaemonClient::connect() {
        Ok(c) => c,
        Err(_) => return handle_not_running(format),
    };

    match command {
        ProjectCommand::List {} => handle_list(&client, format).await,
    }
}

fn handle_not_running(format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => println!("oj daemon: not running"),
        OutputFormat::Json => println!(r#"{{ "status": "not_running" }}"#),
    }
    Ok(())
}

async fn handle_list(client: &DaemonClient, format: OutputFormat) -> Result<()> {
    let projects = match client.list_projects().await {
        Ok(data) => data,
        Err(crate::client::ClientError::DaemonNotRunning) => {
            return handle_not_running(format);
        }
        Err(crate::client::ClientError::Io(ref e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            return handle_not_running(format);
        }
        Err(e) => return Err(e.into()),
    };

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&projects)?);
        }
        OutputFormat::Text => {
            if projects.is_empty() {
                println!("No active projects");
                return Ok(());
            }

            // Compute column widths
            let name_w = projects
                .iter()
                .map(|p| p.name.len())
                .max()
                .unwrap_or(0)
                .max(4);
            let root_w = projects
                .iter()
                .map(|p| {
                    if p.root.as_os_str().is_empty() {
                        9 // "(unknown)"
                    } else {
                        p.root.display().to_string().len()
                    }
                })
                .max()
                .unwrap_or(0)
                .max(4);

            // Print header
            println!(
                "{}  {}  {:>9}  {:>7}  {:>6}  {:>5}",
                color::header(&format!("{:<name_w$}", "NAME", name_w = name_w)),
                color::header(&format!("{:<root_w$}", "ROOT", root_w = root_w)),
                color::header("PIPELINES"),
                color::header("WORKERS"),
                color::header("AGENTS"),
                color::header("CRONS"),
            );

            // Print rows
            for p in &projects {
                let root = if p.root.as_os_str().is_empty() {
                    "(unknown)".to_string()
                } else {
                    p.root.display().to_string()
                };
                println!(
                    "{:<name_w$}  {:<root_w$}  {:>9}  {:>7}  {:>6}  {:>5}",
                    p.name,
                    root,
                    p.active_pipelines,
                    p.workers,
                    p.active_agents,
                    p.crons,
                    name_w = name_w,
                    root_w = root_w,
                );
            }
        }
    }

    Ok(())
}
