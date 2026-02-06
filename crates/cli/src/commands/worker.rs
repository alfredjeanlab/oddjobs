// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker command handlers

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::{ClientKind, DaemonClient};
use crate::color;
use crate::output::{display_log, print_prune_results, print_start_results, OutputFormat};
use crate::table::{project_cell, should_show_project, Column, Table};

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
        /// Worker name from runbook
        name: String,
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
    project_root: &std::path::Path,
    namespace: &str,
    project_filter: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        WorkerCommand::Start { name, all } => {
            if !all && name.is_none() {
                anyhow::bail!("worker name required (or use --all)");
            }
            let worker_name = name.unwrap_or_default();
            let result = client
                .worker_start(project_root, namespace, &worker_name, all)
                .await?;
            print_start_results(&result, "Worker", "workers", namespace);
        }
        WorkerCommand::Stop { name } => {
            client
                .worker_stop(&name, namespace, Some(project_root))
                .await?;
            println!(
                "Worker '{}' stopped ({})",
                color::header(&name),
                color::muted(namespace)
            );
        }
        WorkerCommand::Restart { name } => {
            let worker_name = client
                .worker_restart(project_root, namespace, &name)
                .await?;
            println!("Worker '{}' restarted", color::header(&worker_name));
        }
        WorkerCommand::Resize { name, concurrency } => {
            if concurrency == 0 {
                anyhow::bail!("concurrency must be at least 1");
            }
            let (worker_name, old, new) =
                client.worker_resize(&name, namespace, concurrency).await?;
            println!(
                "Worker '{}' resized: {} → {} ({})",
                color::header(&worker_name),
                old,
                new,
                color::muted(namespace)
            );
        }
        WorkerCommand::Logs {
            name,
            follow,
            limit,
        } => {
            let (log_path, content) = client
                .get_worker_logs(&name, namespace, limit, Some(project_root))
                .await?;
            display_log(&log_path, &content, follow, format, "worker", &name).await?;
        }
        WorkerCommand::List {} => {
            let mut workers = client.list_workers().await?;

            // Filter by explicit --project flag (OJ_NAMESPACE is NOT used for filtering)
            if let Some(proj) = project_filter {
                workers.retain(|w| w.namespace == proj);
            }
            workers.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&workers)?);
                }
                OutputFormat::Text => {
                    if workers.is_empty() {
                        println!("No workers found");
                    } else {
                        let show_project =
                            should_show_project(workers.iter().map(|w| w.namespace.as_str()));

                        let mut cols = vec![Column::left("KIND")];
                        if show_project {
                            cols.push(Column::left("PROJECT"));
                        }
                        cols.extend([
                            Column::left("QUEUE"),
                            Column::status("STATUS"),
                            Column::left("ACTIVE"),
                            Column::left("CONCURRENCY"),
                        ]);
                        let mut table = Table::new(cols);

                        for w in &workers {
                            let mut cells = vec![w.name.clone()];
                            if show_project {
                                cells.push(project_cell(&w.namespace));
                            }
                            cells.extend([
                                w.queue.clone(),
                                w.status.clone(),
                                w.active.to_string(),
                                w.concurrency.to_string(),
                            ]);
                            table.row(cells);
                        }
                        table.render(&mut std::io::stdout());
                    }
                }
            }
        }
        WorkerCommand::Prune { all, dry_run } => {
            let filter_namespace = oj_core::namespace_to_option(namespace);
            let (pruned, skipped) = client.worker_prune(all, dry_run, filter_namespace).await?;

            print_prune_results(
                &pruned,
                skipped,
                dry_run,
                format,
                "worker",
                "skipped",
                |entry| {
                    let ns = if entry.namespace.is_empty() {
                        "(no project)"
                    } else {
                        &entry.namespace
                    };
                    format!("worker '{}' ({})", entry.name, ns)
                },
            )?;
        }
    }
    Ok(())
}
