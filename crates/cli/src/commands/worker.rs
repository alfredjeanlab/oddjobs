// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker command handlers

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::color;
use crate::output::{display_log, print_prune_results, OutputFormat};
use crate::table::{project_cell, should_show_project, Column, Table};

use oj_daemon::{Query, Request, Response};

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
    /// Change a worker's concurrency at runtime
    Resize {
        /// Worker name from runbook
        name: String,
        /// New concurrency value (must be >= 1)
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
            let request = Request::WorkerStart {
                project_root: project_root.to_path_buf(),
                namespace: namespace.to_string(),
                worker_name: name.clone().unwrap_or_default(),
                all,
            };
            match client.send(&request).await? {
                Response::WorkerStarted { worker_name } => {
                    println!(
                        "Worker '{}' started ({})",
                        color::header(&worker_name),
                        color::muted(namespace)
                    );
                }
                Response::WorkersStarted { started, skipped } => {
                    for worker_name in &started {
                        println!(
                            "Worker '{}' started ({})",
                            color::header(worker_name),
                            color::muted(namespace)
                        );
                    }
                    for (worker_name, reason) in &skipped {
                        println!(
                            "Worker '{}' skipped: {}",
                            color::header(worker_name),
                            color::muted(reason)
                        );
                    }
                    if started.is_empty() && skipped.is_empty() {
                        println!("No workers found in runbooks");
                    }
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        WorkerCommand::Stop { name } => {
            let request = Request::WorkerStop {
                worker_name: name.clone(),
                namespace: namespace.to_string(),
                project_root: Some(project_root.to_path_buf()),
            };
            match client.send(&request).await? {
                Response::Ok => {
                    println!(
                        "Worker '{}' stopped ({})",
                        color::header(&name),
                        color::muted(namespace)
                    );
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        WorkerCommand::Restart { name } => {
            let request = Request::WorkerRestart {
                project_root: project_root.to_path_buf(),
                namespace: namespace.to_string(),
                worker_name: name.clone(),
            };
            match client.send(&request).await? {
                Response::WorkerStarted { worker_name } => {
                    println!("Worker '{}' restarted", color::header(&worker_name));
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        WorkerCommand::Resize { name, concurrency } => {
            if concurrency == 0 {
                anyhow::bail!("concurrency must be at least 1");
            }
            let request = Request::WorkerResize {
                worker_name: name.clone(),
                namespace: namespace.to_string(),
                concurrency,
            };
            match client.send(&request).await? {
                Response::WorkerResized {
                    worker_name,
                    concurrency,
                } => {
                    println!(
                        "Worker '{}' resized to concurrency {} ({})",
                        color::header(&worker_name),
                        color::header(&concurrency.to_string()),
                        color::muted(namespace)
                    );
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
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
            let request = Request::Query {
                query: Query::ListWorkers,
            };
            match client.send(&request).await? {
                Response::Workers { mut workers } => {
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
                                let show_project = should_show_project(
                                    workers.iter().map(|w| w.namespace.as_str()),
                                );

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
                Response::Error { message } => anyhow::bail!("{}", message),
                _ => anyhow::bail!("unexpected response from daemon"),
            }
        }
        WorkerCommand::Prune { all, dry_run } => {
            let filter_namespace = if namespace.is_empty() {
                None
            } else {
                Some(namespace)
            };
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
