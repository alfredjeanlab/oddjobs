// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron command handlers

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::output::{display_log, OutputFormat};

use oj_daemon::{Query, Request, Response};

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
        /// Cron name from runbook
        name: String,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// Stop a cron (cancels interval timer)
    Stop {
        /// Cron name from runbook
        name: String,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// Run the cron's pipeline once now (ignores interval)
    Once {
        /// Cron name from runbook
        name: String,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
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
        /// Project namespace override
        #[arg(long)]
        project: Option<String>,
    },
}

pub async fn handle(
    command: CronCommand,
    client: &DaemonClient,
    project_root: &std::path::Path,
    namespace: &str,
    format: OutputFormat,
) -> Result<()> {
    match command {
        CronCommand::Start { name, project } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok())
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::CronStart {
                project_root: project_root.to_path_buf(),
                namespace: effective_namespace,
                cron_name: name,
            };
            match client.send(&request).await? {
                Response::CronStarted { cron_name } => {
                    println!("Cron '{}' started", cron_name);
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        CronCommand::Stop { name, project } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok())
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::CronStop {
                cron_name: name.clone(),
                namespace: effective_namespace,
            };
            match client.send(&request).await? {
                Response::Ok => {
                    println!("Cron '{}' stopped", name);
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        CronCommand::Once { name, project } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok())
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::CronOnce {
                project_root: project_root.to_path_buf(),
                namespace: effective_namespace,
                cron_name: name,
            };
            match client.send(&request).await? {
                Response::CommandStarted {
                    pipeline_id,
                    pipeline_name,
                } => {
                    println!("Pipeline '{}' started ({})", pipeline_name, pipeline_id);
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        CronCommand::Logs {
            name,
            follow,
            limit,
            project,
        } => {
            let _effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok())
                .unwrap_or_else(|| namespace.to_string());
            let (log_path, content) = client.get_cron_logs(&name, limit).await?;
            display_log(&log_path, &content, follow, format, "cron", &name).await?;
        }
        CronCommand::List {} => {
            let request = Request::Query {
                query: Query::ListCrons,
            };
            match client.send(&request).await? {
                Response::Crons { mut crons } => {
                    crons.sort_by(|a, b| a.name.cmp(&b.name));
                    match format {
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&crons)?);
                        }
                        OutputFormat::Text => {
                            if crons.is_empty() {
                                println!("No crons found");
                            } else {
                                // Determine whether to show PROJECT column
                                let namespaces: std::collections::HashSet<&str> =
                                    crons.iter().map(|c| c.namespace.as_str()).collect();
                                let show_project = namespaces.len() > 1
                                    || namespaces.iter().any(|n| !n.is_empty());

                                // Compute dynamic column widths from data
                                let name_w =
                                    crons.iter().map(|c| c.name.len()).max().unwrap_or(4).max(4);
                                let no_project = "(no project)";
                                let proj_w = if show_project {
                                    crons
                                        .iter()
                                        .map(|c| {
                                            if c.namespace.is_empty() {
                                                no_project.len()
                                            } else {
                                                c.namespace.len()
                                            }
                                        })
                                        .max()
                                        .unwrap_or(7)
                                        .max(7)
                                } else {
                                    0
                                };
                                let interval_w = crons
                                    .iter()
                                    .map(|c| c.interval.len())
                                    .max()
                                    .unwrap_or(8)
                                    .max(8);
                                let pipeline_w = crons
                                    .iter()
                                    .map(|c| c.pipeline.len())
                                    .max()
                                    .unwrap_or(8)
                                    .max(8);

                                if show_project {
                                    println!(
                                        "{:<name_w$} {:<proj_w$} {:<interval_w$} {:<pipeline_w$} STATUS",
                                        "NAME", "PROJECT", "INTERVAL", "PIPELINE",
                                    );
                                } else {
                                    println!(
                                        "{:<name_w$} {:<interval_w$} {:<pipeline_w$} STATUS",
                                        "NAME", "INTERVAL", "PIPELINE",
                                    );
                                }
                                for c in &crons {
                                    if show_project {
                                        let proj = if c.namespace.is_empty() {
                                            no_project
                                        } else {
                                            &c.namespace
                                        };
                                        println!(
                                            "{:<name_w$} {:<proj_w$} {:<interval_w$} {:<pipeline_w$} {}",
                                            c.name, proj, c.interval, c.pipeline, c.status,
                                        );
                                    } else {
                                        println!(
                                            "{:<name_w$} {:<interval_w$} {:<pipeline_w$} {}",
                                            c.name, c.interval, c.pipeline, c.status,
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
    }
    Ok(())
}
