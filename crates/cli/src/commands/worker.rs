// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker command handlers

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::output::{display_log, OutputFormat};
use crate::table::{Column, Table};

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
        /// Worker name from runbook
        name: String,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// Stop a worker (active pipelines continue, no new items dispatched)
    Stop {
        /// Worker name from runbook
        name: String,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// Restart a worker (stop, reload runbook, start)
    Restart {
        /// Worker name from runbook
        name: String,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
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
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// List all workers and their status
    List {
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// Remove stopped workers from daemon state
    Prune {
        /// Prune all stopped workers (currently same as default)
        #[arg(long)]
        all: bool,

        /// Show what would be pruned without making changes
        #[arg(long)]
        dry_run: bool,

        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
}

pub async fn handle(
    command: WorkerCommand,
    client: &DaemonClient,
    project_root: &std::path::Path,
    namespace: &str,
    format: OutputFormat,
) -> Result<()> {
    match command {
        WorkerCommand::Start { name, project } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            // (empty OJ_NAMESPACE treated as unset)
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::WorkerStart {
                project_root: project_root.to_path_buf(),
                namespace: effective_namespace.clone(),
                worker_name: name.clone(),
            };
            match client.send(&request).await? {
                Response::WorkerStarted { worker_name } => {
                    println!("Worker '{}' started ({})", worker_name, effective_namespace);
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        WorkerCommand::Stop { name, project } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            // (empty OJ_NAMESPACE treated as unset)
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::WorkerStop {
                worker_name: name.clone(),
                namespace: effective_namespace.clone(),
                project_root: Some(project_root.to_path_buf()),
            };
            match client.send(&request).await? {
                Response::Ok => {
                    println!("Worker '{}' stopped ({})", name, effective_namespace);
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        WorkerCommand::Restart { name, project } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            // (empty OJ_NAMESPACE treated as unset)
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::WorkerRestart {
                project_root: project_root.to_path_buf(),
                namespace: effective_namespace,
                worker_name: name.clone(),
            };
            match client.send(&request).await? {
                Response::WorkerStarted { worker_name } => {
                    println!("Worker '{}' restarted", worker_name);
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
            project,
        } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            // (empty OJ_NAMESPACE treated as unset)
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| namespace.to_string());

            let (log_path, content) = client
                .get_worker_logs(&name, &effective_namespace, limit, Some(project_root))
                .await?;
            display_log(&log_path, &content, follow, format, "worker", &name).await?;
        }
        WorkerCommand::List { project } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env (empty OJ_NAMESPACE treated as unset)
            let filter_namespace =
                project.or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()));

            let request = Request::Query {
                query: Query::ListWorkers,
            };
            match client.send(&request).await? {
                Response::Workers { mut workers } => {
                    // Filter by namespace if --project was specified or OJ_NAMESPACE is set
                    if let Some(ref ns) = filter_namespace {
                        workers.retain(|w| w.namespace == *ns);
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
                                // Determine whether to show PROJECT column
                                let namespaces: std::collections::HashSet<&str> =
                                    workers.iter().map(|w| w.namespace.as_str()).collect();
                                let show_project = namespaces.len() > 1
                                    || namespaces.iter().any(|n| !n.is_empty());

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
                                        let proj = if w.namespace.is_empty() {
                                            "(no project)".to_string()
                                        } else {
                                            w.namespace.clone()
                                        };
                                        cells.push(proj);
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
        WorkerCommand::Prune {
            all,
            dry_run,
            project,
        } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env (empty treated as unset)
            let filter_namespace =
                project.or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()));

            let (pruned, skipped) = client
                .worker_prune(all, dry_run, filter_namespace.as_deref())
                .await?;

            match format {
                OutputFormat::Text => {
                    if dry_run {
                        println!("Dry run — no changes made\n");
                    }

                    for entry in &pruned {
                        let label = if dry_run { "Would prune" } else { "Pruned" };
                        let ns = if entry.namespace.is_empty() {
                            "(no project)".to_string()
                        } else {
                            entry.namespace.clone()
                        };
                        println!("{} worker '{}' ({})", label, entry.name, ns);
                    }

                    let verb = if dry_run { "would be pruned" } else { "pruned" };
                    println!("\n{} worker(s) {}, {} skipped", pruned.len(), verb, skipped);
                }
                OutputFormat::Json => {
                    let obj = serde_json::json!({
                        "dry_run": dry_run,
                        "pruned": pruned,
                        "skipped": skipped,
                    });
                    println!("{}", serde_json::to_string_pretty(&obj)?);
                }
            }
        }
    }
    Ok(())
}
