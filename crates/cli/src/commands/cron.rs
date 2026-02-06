// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron command handlers

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::{ClientKind, DaemonClient};
use crate::output::{display_log, print_prune_results, print_start_results, OutputFormat};
use crate::table::{project_cell, should_show_project, Column, Table};

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
        /// Cron name from runbook
        name: String,
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
    project_root: &std::path::Path,
    namespace: &str,
    project_filter: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        CronCommand::Start { name, all } => {
            if !all && name.is_none() {
                anyhow::bail!("cron name required (or use --all)");
            }
            let cron_name = name.unwrap_or_default();
            let result = client
                .cron_start(project_root, namespace, &cron_name, all)
                .await?;
            print_start_results(&result, "Cron", "crons", namespace);
        }
        CronCommand::Stop { name } => {
            client
                .cron_stop(&name, namespace, Some(project_root))
                .await?;
            println!("Cron '{}' stopped ({})", name, namespace);
        }
        CronCommand::Restart { name } => {
            let cron_name = client.cron_restart(project_root, namespace, &name).await?;
            println!("Cron '{}' restarted", cron_name);
        }
        CronCommand::Once { name } => {
            let (job_id, job_name) = client.cron_once(project_root, namespace, &name).await?;
            println!("Job '{}' started ({})", job_name, job_id);
        }
        CronCommand::Logs {
            name,
            follow,
            limit,
        } => {
            let (log_path, content) = client
                .get_cron_logs(&name, namespace, limit, Some(project_root))
                .await?;
            display_log(&log_path, &content, follow, format, "cron", &name).await?;
        }
        CronCommand::Prune { all, dry_run } => {
            let (mut pruned, skipped) = client.cron_prune(all, dry_run).await?;

            // Filter by project namespace
            if !namespace.is_empty() {
                pruned.retain(|e| e.namespace == namespace);
            }

            print_prune_results(
                &pruned,
                skipped,
                dry_run,
                format,
                "cron",
                "skipped",
                |entry| {
                    let ns = if entry.namespace.is_empty() {
                        "(no project)"
                    } else {
                        &entry.namespace
                    };
                    format!("cron '{}' ({})", entry.name, ns)
                },
            )?;
        }
        CronCommand::List {} => {
            let mut crons = client.list_crons().await?;

            // Filter by explicit --project flag (OJ_NAMESPACE is NOT used for filtering)
            if let Some(proj) = project_filter {
                crons.retain(|c| c.namespace == proj);
            }
            crons.sort_by(|a, b| a.name.cmp(&b.name));
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&crons)?);
                }
                OutputFormat::Text => {
                    if crons.is_empty() {
                        println!("No crons found");
                    } else {
                        let show_project =
                            should_show_project(crons.iter().map(|c| c.namespace.as_str()));

                        let mut cols = vec![Column::left("KIND")];
                        if show_project {
                            cols.push(Column::left("PROJECT"));
                        }
                        cols.extend([
                            Column::left("INTERVAL"),
                            Column::left("JOB"),
                            Column::left("TIME"),
                            Column::status("STATUS"),
                        ]);
                        let mut table = Table::new(cols);

                        for c in &crons {
                            let mut cells = vec![c.name.clone()];
                            if show_project {
                                cells.push(project_cell(&c.namespace));
                            }
                            cells.extend([
                                c.interval.clone(),
                                c.job.clone(),
                                c.time.clone(),
                                c.status.clone(),
                            ]);
                            table.row(cells);
                        }
                        table.render(&mut std::io::stdout());
                    }
                }
            }
        }
    }
    Ok(())
}
