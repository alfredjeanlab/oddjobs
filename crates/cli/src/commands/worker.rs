// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Worker command handlers

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::output::OutputFormat;

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
    format: OutputFormat,
) -> Result<()> {
    match command {
        WorkerCommand::Start { name } => {
            let request = Request::WorkerStart {
                project_root: project_root.to_path_buf(),
                namespace: namespace.to_string(),
                worker_name: name.clone(),
            };
            match client.send(&request).await? {
                Response::WorkerStarted { worker_name } => {
                    println!("Worker '{}' started", worker_name);
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        WorkerCommand::List {} => {
            let request = Request::Query {
                query: Query::ListWorkers,
            };
            match client.send(&request).await? {
                Response::Workers { mut workers } => {
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

                                // Compute dynamic column widths from data
                                let name_w = workers
                                    .iter()
                                    .map(|w| w.name.len())
                                    .max()
                                    .unwrap_or(4)
                                    .max(4);
                                let proj_w = if show_project {
                                    workers
                                        .iter()
                                        .map(|w| w.namespace.len())
                                        .max()
                                        .unwrap_or(7)
                                        .max(7)
                                } else {
                                    0
                                };
                                let queue_w = workers
                                    .iter()
                                    .map(|w| w.queue.len())
                                    .max()
                                    .unwrap_or(5)
                                    .max(5);
                                let status_w = workers
                                    .iter()
                                    .map(|w| w.status.len())
                                    .max()
                                    .unwrap_or(6)
                                    .max(6);
                                let active_w = 6; // "ACTIVE"

                                if show_project {
                                    println!(
                                        "{:<name_w$} {:<proj_w$} {:<queue_w$} {:<status_w$} {:<active_w$} CONCURRENCY",
                                        "NAME", "PROJECT", "QUEUE", "STATUS", "ACTIVE",
                                    );
                                } else {
                                    println!(
                                        "{:<name_w$} {:<queue_w$} {:<status_w$} {:<active_w$} CONCURRENCY",
                                        "NAME", "QUEUE", "STATUS", "ACTIVE",
                                    );
                                }
                                for w in &workers {
                                    if show_project {
                                        println!(
                                            "{:<name_w$} {:<proj_w$} {:<queue_w$} {:<status_w$} {:<active_w$} {}",
                                            &w.name[..w.name.len().min(name_w)],
                                            &w.namespace[..w.namespace.len().min(proj_w)],
                                            &w.queue[..w.queue.len().min(queue_w)],
                                            &w.status,
                                            w.active,
                                            w.concurrency,
                                        );
                                    } else {
                                        println!(
                                            "{:<name_w$} {:<queue_w$} {:<status_w$} {:<active_w$} {}",
                                            &w.name[..w.name.len().min(name_w)],
                                            &w.queue[..w.queue.len().min(queue_w)],
                                            &w.status,
                                            w.active,
                                            w.concurrency,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                Response::Error { message } => anyhow::bail!("{}", message),
                _ => anyhow::bail!("unexpected response from daemon"),
            }
        }
        WorkerCommand::Prune { all, dry_run } => {
            let (pruned, skipped) = client.worker_prune(all, dry_run).await?;

            match format {
                OutputFormat::Text => {
                    if dry_run {
                        println!("Dry run — no changes made\n");
                    }

                    for entry in &pruned {
                        let label = if dry_run { "Would prune" } else { "Pruned" };
                        let ns = if entry.namespace.is_empty() {
                            "(default)".to_string()
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
