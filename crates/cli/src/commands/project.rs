// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj project` — project management commands.

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::output::OutputFormat;
use crate::table::{Column, Table};

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

            let mut table = Table::new(vec![
                Column::left("NAME"),
                Column::left("ROOT"),
                Column::right("PIPELINES"),
                Column::right("WORKERS"),
                Column::right("AGENTS"),
                Column::right("CRONS"),
            ]);
            for p in &projects {
                let root = if p.root.as_os_str().is_empty() {
                    "(unknown)".to_string()
                } else {
                    p.root.display().to_string()
                };
                table.row(vec![
                    p.name.clone(),
                    root,
                    p.active_pipelines.to_string(),
                    p.workers.to_string(),
                    p.active_agents.to_string(),
                    p.crons.to_string(),
                ]);
            }
            table.render(&mut std::io::stdout());
        }
    }

    Ok(())
}
