// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker command handlers

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::{ClientKind, DaemonClient};
use crate::color;
use crate::output::{
    display_log, filter_by_project, handle_list, poll_log_follow, print_prune_results,
    print_start_results, print_stop_results, require_name_or_all, OutputFormat,
};
use crate::table::{Column, Table};

#[derive(Args)]
pub struct WorkerArgs {
    #[command(subcommand)]
    pub command: WorkerCommand,
}

#[derive(Subcommand)]
pub enum WorkerCommand {
    /// Start a worker (idempotent: wakes it if already running)
    Start {
        /// Worker name from runbook (required unless --all)
        name: Option<String>,
        /// Start all workers defined in runbooks
        #[arg(long)]
        all: bool,
    },
    /// Stop a worker (active jobs continue, no new items dispatched)
    Stop {
        /// Worker name from runbook (required unless --all)
        name: Option<String>,
        /// Stop all running workers
        #[arg(long)]
        all: bool,
    },
    /// Restart a worker (stop, reload runbook, start)
    Restart {
        /// Worker name from runbook
        name: String,
    },
    /// Resize a worker's concurrency limit at runtime
    Resize {
        /// Worker name from runbook
        name: String,
        /// New concurrency limit (must be > 0)
        concurrency: u32,
    },
    /// View worker activity log
    Logs {
        /// Worker name
        name: String,
        /// Stream live activity (like tail -f)
        #[arg(long, short)]
        follow: bool,
        /// Number of recent lines to show (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
    },
    /// List all workers and their status
    List {},
    /// Remove stopped workers from daemon state
    Prune {
        /// Prune all stopped workers (currently same as default)
        #[arg(long)]
        all: bool,

        /// Show what would be pruned without making changes
        #[arg(long)]
        dry_run: bool,
    },
}

impl WorkerCommand {
    pub fn client_kind(&self) -> ClientKind {
        match self {
            Self::List {} | Self::Logs { .. } => ClientKind::Query,
            _ => ClientKind::Action,
        }
    }
}

pub async fn handle(
    command: WorkerCommand,
    client: &DaemonClient,
    project_path: &std::path::Path,
    project: &str,
    project_filter: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        WorkerCommand::Start { name, all } => {
            let worker_name = require_name_or_all(name, all, "worker")?;
            let result = client.worker_start(project_path, project, &worker_name, all).await?;
            print_start_results(&result, "Worker", "workers", project);
        }
        WorkerCommand::Stop { name, all } => {
            let worker_name = require_name_or_all(name, all, "worker")?;
            let result = client.worker_stop(&worker_name, project, Some(project_path), all).await?;
            print_stop_results(&result, "Worker", "workers", project);
        }
        WorkerCommand::Restart { name } => {
            let worker_name = client.worker_restart(project_path, project, &name).await?;
            println!("Worker '{}' restarted", color::header(&worker_name));
        }
        WorkerCommand::Resize { name, concurrency } => {
            if concurrency == 0 {
                anyhow::bail!("concurrency must be at least 1");
            }
            let (worker_name, old, new) = client.worker_resize(&name, project, concurrency).await?;
            println!(
                "Worker '{}' resized: {} → {} ({})",
                color::header(&worker_name),
                old,
                new,
                color::muted(project)
            );
        }
        WorkerCommand::Logs { name, follow, limit } => {
            let (log_path, content, offset) =
                client.get_worker_logs(&name, project, limit, 0, Some(project_path)).await?;
            if let Some(off) =
                display_log(&log_path, &content, follow, offset, format, "worker", &name).await?
            {
                let name = name.clone();
                let ns = project.to_string();
                poll_log_follow(off, |o| {
                    let name = name.clone();
                    let ns = ns.clone();
                    async move {
                        let (_, c, new_off) =
                            client.get_worker_logs(&name, &ns, 0, o, None).await?;
                        Ok((c, new_off))
                    }
                })
                .await?;
            }
        }
        WorkerCommand::List {} => {
            let mut workers = client.list_workers().await?;
            filter_by_project(&mut workers, project_filter, |w| &w.project);
            workers.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
            handle_list(format, &workers, "No workers found", |items, out| {
                let cols = vec![
                    Column::left("KIND"),
                    Column::left("PROJECT"),
                    Column::left("QUEUE"),
                    Column::status("STATUS"),
                    Column::left("ACTIVE"),
                    Column::left("CONCURRENCY"),
                ];
                let mut table = Table::new(cols);
                for w in items {
                    let ns = if w.project.is_empty() { "-" } else { &w.project };
                    let cells = vec![
                        w.name.clone(),
                        ns.to_string(),
                        w.queue.clone(),
                        w.status.clone(),
                        w.active.to_string(),
                        w.concurrency.to_string(),
                    ];
                    table.row(cells);
                }
                table.render(out);
            })?;
        }
        WorkerCommand::Prune { all, dry_run } => {
            let filter_namespace = oj_core::namespace_to_option(project);
            let (pruned, skipped) = client.worker_prune(all, dry_run, filter_namespace).await?;

            print_prune_results(&pruned, skipped, dry_run, format, "worker", "skipped", |entry| {
                let ns = if entry.project.is_empty() { "(no project)" } else { &entry.project };
                format!("worker '{}' ({})", entry.name, ns)
            })?;
        }
    }
    Ok(())
}
