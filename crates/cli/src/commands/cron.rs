// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cron command handlers

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::output::{display_log, OutputFormat};
use crate::table::{Column, Table};

use oj_daemon::{Query, Request, Response};

#[derive(Args)]
pub struct CronArgs {
    #[command(subcommand)]
    pub command: CronCommand,
}

#[derive(Subcommand)]
pub enum CronCommand {
    /// List all crons and their status
    List {
        /// Filter by project namespace
        #[arg(long = "project")]
        project: Option<String>,
    },
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
    /// Restart a cron (stop, reload runbook, start)
    Restart {
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
    /// Remove stopped crons from daemon state
    Prune {
        /// Prune all stopped crons (currently same as default)
        #[arg(long)]
        all: bool,

        /// Show what would be pruned without making changes
        #[arg(long)]
        dry_run: bool,

        /// Filter by project namespace
        #[arg(long = "project")]
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
            // (empty OJ_NAMESPACE treated as unset)
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::CronStart {
                project_root: project_root.to_path_buf(),
                namespace: effective_namespace.clone(),
                cron_name: name,
            };
            match client.send(&request).await? {
                Response::CronStarted { cron_name } => {
                    println!("Cron '{}' started ({})", cron_name, effective_namespace);
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
            // (empty OJ_NAMESPACE treated as unset)
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::CronStop {
                cron_name: name.clone(),
                namespace: effective_namespace.clone(),
                project_root: Some(project_root.to_path_buf()),
            };
            match client.send(&request).await? {
                Response::Ok => {
                    println!("Cron '{}' stopped ({})", name, effective_namespace);
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        CronCommand::Restart { name, project } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            // (empty OJ_NAMESPACE treated as unset)
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::CronRestart {
                project_root: project_root.to_path_buf(),
                namespace: effective_namespace,
                cron_name: name.clone(),
            };
            match client.send(&request).await? {
                Response::CronStarted { cron_name } => {
                    println!("Cron '{}' restarted", cron_name);
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
            // (empty OJ_NAMESPACE treated as unset)
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
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
            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            // (empty OJ_NAMESPACE treated as unset)
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| namespace.to_string());
            let (log_path, content) = client
                .get_cron_logs(&name, &effective_namespace, limit, Some(project_root))
                .await?;
            display_log(&log_path, &content, follow, format, "cron", &name).await?;
        }
        CronCommand::Prune {
            all,
            dry_run,
            project,
        } => {
            let (mut pruned, skipped) = client.cron_prune(all, dry_run).await?;

            // Filter by project namespace (empty OJ_NAMESPACE treated as unset)
            let filter_namespace =
                project.or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()));
            if let Some(ref ns) = filter_namespace {
                pruned.retain(|e| e.namespace == *ns);
            }

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
                        println!("{} cron '{}' ({})", label, entry.name, ns);
                    }

                    let verb = if dry_run { "would be pruned" } else { "pruned" };
                    println!("\n{} cron(s) {}, {} skipped", pruned.len(), verb, skipped);
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
        CronCommand::List { project } => {
            let request = Request::Query {
                query: Query::ListCrons,
            };
            match client.send(&request).await? {
                Response::Crons { mut crons } => {
                    // Filter by project namespace (empty OJ_NAMESPACE treated as unset)
                    let filter_namespace = project
                        .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()));
                    if let Some(ref ns) = filter_namespace {
                        crons.retain(|c| c.namespace == *ns);
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
                                // Determine whether to show PROJECT column
                                let namespaces: std::collections::HashSet<&str> =
                                    crons.iter().map(|c| c.namespace.as_str()).collect();
                                let show_project = namespaces.len() > 1
                                    || namespaces.iter().any(|n| !n.is_empty());

                                let mut cols = vec![Column::left("KIND")];
                                if show_project {
                                    cols.push(Column::left("PROJECT"));
                                }
                                cols.extend([
                                    Column::left("INTERVAL"),
                                    Column::left("PIPELINE"),
                                    Column::left("TIME"),
                                    Column::status("STATUS"),
                                ]);
                                let mut table = Table::new(cols);

                                for c in &crons {
                                    let mut cells = vec![c.name.clone()];
                                    if show_project {
                                        let proj = if c.namespace.is_empty() {
                                            "(no project)".to_string()
                                        } else {
                                            c.namespace.clone()
                                        };
                                        cells.push(proj);
                                    }
                                    cells.extend([
                                        c.interval.clone(),
                                        c.pipeline.clone(),
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
                Response::Error { message } => anyhow::bail!("{}", message),
                _ => anyhow::bail!("unexpected response from daemon"),
            }
        }
    }
    Ok(())
}
