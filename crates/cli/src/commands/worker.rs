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
                    workers.sort_by(|a, b| a.name.cmp(&b.name));
                    match format {
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&workers)?);
                        }
                        OutputFormat::Text => {
                            if workers.is_empty() {
                                println!("No workers found");
                            } else {
                                println!(
                                    "{:<20} {:<15} {:<10} {:<8} CONCURRENCY",
                                    "NAME", "QUEUE", "STATUS", "ACTIVE"
                                );
                                for w in &workers {
                                    println!(
                                        "{:<20} {:<15} {:<10} {:<8} {}",
                                        &w.name[..w.name.len().min(20)],
                                        &w.queue[..w.queue.len().min(15)],
                                        &w.status,
                                        w.active,
                                        w.concurrency,
                                    );
                                }
                            }
                        }
                    }
                }
                Response::Error { message } => anyhow::bail!("{}", message),
                _ => anyhow::bail!("unexpected response from daemon"),
            }
        }
    }
    Ok(())
}
