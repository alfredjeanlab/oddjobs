// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron command handlers

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::{ClientKind, DaemonClient};
use crate::output::{
    display_log, filter_by_project, handle_list, poll_log_follow, print_prune_results,
    print_start_results, print_stop_results, require_name_or_all, OutputFormat,
};
use crate::table::{Column, Table};

#[derive(Args)]
pub struct CronArgs {
    #[command(subcommand)]
    pub command: CronCommand,
}

#[derive(Subcommand)]
pub enum CronCommand {
    /// List all crons and their status
    List {},
    /// Start a cron (begins interval timer)
    Start {
        /// Cron name from runbook (required unless --all)
        name: Option<String>,
        /// Start all crons defined in runbooks
        #[arg(long)]
        all: bool,
    },
    /// Stop a cron (cancels interval timer)
    Stop {
        /// Cron name from runbook (required unless --all)
        name: Option<String>,
        /// Stop all running crons
        #[arg(long)]
        all: bool,
    },
    /// Restart a cron (stop, reload runbook, start)
    Restart {
        /// Cron name from runbook
        name: String,
    },
    /// Run the cron's job once now (ignores interval)
    Once {
        /// Cron name from runbook
        name: String,
    },
    /// View cron activity log
    Logs {
        /// Cron name from runbook
        name: String,
        /// Stream live activity (like tail -f)
        #[arg(long, short)]
        follow: bool,
        /// Number of recent lines to show (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
    },
    /// Remove stopped crons from daemon state
    Prune {
        /// Prune all stopped crons (currently same as default)
        #[arg(long)]
        all: bool,

        /// Show what would be pruned without making changes
        #[arg(long)]
        dry_run: bool,
    },
}

impl CronCommand {
    pub fn client_kind(&self) -> ClientKind {
        match self {
            Self::List {} | Self::Logs { .. } => ClientKind::Query,
            _ => ClientKind::Action,
        }
    }
}

pub async fn handle(
    command: CronCommand,
    client: &DaemonClient,
    project_path: &std::path::Path,
    project: &str,
    project_filter: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        CronCommand::Start { name, all } => {
            let cron_name = require_name_or_all(name, all, "cron")?;
            let result = client.cron_start(project_path, project, &cron_name, all).await?;
            print_start_results(&result, "Cron", "crons", project);
        }
        CronCommand::Stop { name, all } => {
            let cron_name = require_name_or_all(name, all, "cron")?;
            let result = client.cron_stop(&cron_name, project, Some(project_path), all).await?;
            print_stop_results(&result, "Cron", "crons", project);
        }
        CronCommand::Restart { name } => {
            let cron_name = client.cron_restart(project_path, project, &name).await?;
            println!("Cron '{}' restarted", cron_name);
        }
        CronCommand::Once { name } => {
            let (job_id, job_name) = client.cron_once(project_path, project, &name).await?;
            println!("Job '{}' started ({})", job_name, job_id);
        }
        CronCommand::Logs { name, follow, limit } => {
            let (log_path, content, offset) =
                client.get_cron_logs(&name, project, limit, 0, Some(project_path)).await?;
            if let Some(off) =
                display_log(&log_path, &content, follow, offset, format, "cron", &name).await?
            {
                let name = name.clone();
                let ns = project.to_string();
                poll_log_follow(off, |o| {
                    let name = name.clone();
                    let ns = ns.clone();
                    async move {
                        let (_, c, new_off) = client.get_cron_logs(&name, &ns, 0, o, None).await?;
                        Ok((c, new_off))
                    }
                })
                .await?;
            }
        }
        CronCommand::Prune { all, dry_run } => {
            let (mut pruned, skipped) = client.cron_prune(all, dry_run).await?;

            // Filter by project project
            if !project.is_empty() {
                pruned.retain(|e| e.project == project);
            }

            print_prune_results(&pruned, skipped, dry_run, format, "cron", "skipped", |entry| {
                let ns = if entry.project.is_empty() { "(no project)" } else { &entry.project };
                format!("cron '{}' ({})", entry.name, ns)
            })?;
        }
        CronCommand::List {} => {
            let mut crons = client.list_crons().await?;
            filter_by_project(&mut crons, project_filter, |c| &c.project);
            crons.sort_by(|a, b| a.name.cmp(&b.name));
            handle_list(format, &crons, "No crons found", |items, out| {
                let cols = vec![
                    Column::left("KIND"),
                    Column::left("PROJECT"),
                    Column::left("INTERVAL"),
                    Column::left("TARGET"),
                    Column::left("TIME"),
                    Column::status("STATUS"),
                ];
                let mut table = Table::new(cols);
                for c in items {
                    let ns = if c.project.is_empty() { "-" } else { &c.project };
                    let cells = vec![
                        c.name.clone(),
                        ns.to_string(),
                        c.interval.clone(),
                        c.target.clone(),
                        c.time.clone(),
                        c.status.clone(),
                    ];
                    table.row(cells);
                }
                table.render(out);
            })?;
        }
    }
    Ok(())
}
