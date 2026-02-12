// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj project` — project management commands.

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::output::{self, OutputFormat};
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
    let obj = serde_json::json!({ "status": "not_running" });
    output::format_or_json(format, &obj, || println!("oj daemon: not running"))
}

async fn handle_list(client: &DaemonClient, format: OutputFormat) -> Result<()> {
    let projects = match client.list_projects().await {
        Ok(data) => data,
        Err(e) if e.is_not_running() => return handle_not_running(format),
        Err(e) => return Err(e.into()),
    };

    output::handle_list(format, &projects, "No active projects", |items, out| {
        let mut table = Table::new(vec![
            Column::left("NAME"),
            Column::left("ROOT"),
            Column::right("JOBS"),
            Column::right("WORKERS"),
            Column::right("AGENTS"),
            Column::right("CRONS"),
        ]);
        for p in items {
            let root = if p.root.as_os_str().is_empty() {
                "(unknown)".to_string()
            } else {
                p.root.display().to_string()
            };
            table.row(vec![
                p.name.clone(),
                root,
                p.active_jobs.to_string(),
                p.workers.to_string(),
                p.active_agents.to_string(),
                p.crons.to_string(),
            ]);
        }
        table.render(out);
    })
}
