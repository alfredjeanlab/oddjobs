// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue command handlers

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::Path;

use oj_daemon::{Query, Request, Response};

use crate::client::DaemonClient;
use crate::output::{display_log, OutputFormat};

#[derive(Args)]
pub struct QueueArgs {
    #[command(subcommand)]
    pub command: QueueCommand,
}

#[derive(Subcommand)]
pub enum QueueCommand {
    /// Push an item to a queue (or trigger a poll for external queues)
    Push {
        /// Queue name
        queue: String,
        /// Item data as JSON object (optional if --var is provided)
        data: Option<String>,
        /// Item variables (can be repeated: --var key=value)
        #[arg(long = "var", value_parser = parse_key_value)]
        var: Vec<(String, String)>,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// List all known queues
    List {
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// Show items in a specific queue
    Items {
        /// Queue name
        queue: String,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// Remove an item from a persisted queue
    Drop {
        /// Queue name
        queue: String,
        /// Item ID (or prefix)
        item_id: String,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// View queue activity log
    Logs {
        /// Queue name
        queue: String,
        /// Stream live activity (like tail -f)
        #[arg(long, short = 'f')]
        follow: bool,
        /// Number of recent lines to show (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// Retry a dead or failed queue item
    Retry {
        /// Queue name
        queue: String,
        /// Item ID (or prefix)
        item_id: String,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
}

/// Parse a key=value string for --var arguments.
fn parse_key_value(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid input format '{}': must be key=value", s))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

/// Build a JSON object from optional JSON string and --var key=value pairs.
fn build_data_map(data: Option<String>, var: Vec<(String, String)>) -> Result<serde_json::Value> {
    // Start with JSON data if provided
    let mut map = match data {
        Some(json_str) => {
            let val: serde_json::Value = serde_json::from_str(&json_str)
                .map_err(|e| anyhow::anyhow!("invalid JSON data: {}", e))?;
            match val {
                serde_json::Value::Object(m) => m,
                _ => anyhow::bail!("JSON data must be an object"),
            }
        }
        None => serde_json::Map::new(),
    };

    // Merge --var entries (overrides JSON on conflict)
    for (k, v) in var {
        map.insert(k, serde_json::Value::String(v));
    }

    if map.is_empty() {
        anyhow::bail!("no data provided: use --var key=value or pass a JSON object");
    }

    Ok(serde_json::Value::Object(map))
}

pub async fn handle(
    command: QueueCommand,
    client: &DaemonClient,
    project_root: &Path,
    namespace: &str,
    format: OutputFormat,
) -> Result<()> {
    match command {
        QueueCommand::Push {
            queue,
            data,
            var,
            project,
        } => {
            // Build data map; allow empty data for external queues (triggers poll)
            let json_data = if data.is_none() && var.is_empty() {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                build_data_map(data, var)?
            };

            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok())
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::QueuePush {
                project_root: project_root.to_path_buf(),
                namespace: effective_namespace,
                queue_name: queue.clone(),
                data: json_data,
            };

            match client.send(&request).await? {
                Response::QueuePushed {
                    queue_name,
                    item_id,
                } => {
                    println!("Pushed item '{}' to queue '{}'", item_id, queue_name);
                }
                Response::Ok => {
                    println!("Refreshed external queue '{}'", queue);
                }
                Response::Error { message } => {
                    anyhow::bail!("{}", message);
                }
                _ => {
                    anyhow::bail!("unexpected response from daemon");
                }
            }
        }
        QueueCommand::Drop {
            queue,
            item_id,
            project,
        } => {
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok())
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::QueueDrop {
                project_root: project_root.to_path_buf(),
                namespace: effective_namespace,
                queue_name: queue.clone(),
                item_id: item_id.clone(),
            };

            match client.send(&request).await? {
                Response::QueueDropped {
                    queue_name,
                    item_id,
                } => {
                    println!(
                        "Dropped item {} from queue {}",
                        &item_id[..8.min(item_id.len())],
                        queue_name
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
        QueueCommand::Retry {
            queue,
            item_id,
            project,
        } => {
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok())
                .unwrap_or_else(|| namespace.to_string());

            let request = Request::QueueRetry {
                project_root: project_root.to_path_buf(),
                namespace: effective_namespace,
                queue_name: queue.clone(),
                item_id: item_id.clone(),
            };

            match client.send(&request).await? {
                Response::QueueRetried {
                    queue_name,
                    item_id,
                } => {
                    println!(
                        "Retrying item {} in queue {}",
                        &item_id[..8.min(item_id.len())],
                        queue_name
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
        QueueCommand::Logs {
            queue,
            follow,
            limit,
            project,
        } => {
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok())
                .unwrap_or_else(|| namespace.to_string());

            let (log_path, content) = client
                .get_queue_logs(&queue, &effective_namespace, limit)
                .await?;
            display_log(&log_path, &content, follow, format, "queue", &queue).await?;
        }
        QueueCommand::List { project } => {
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok())
                .unwrap_or_else(|| namespace.to_string());
            let request = Request::Query {
                query: Query::ListQueues {
                    project_root: project_root.to_path_buf(),
                    namespace: effective_namespace,
                },
            };
            match client.send(&request).await? {
                Response::Queues { queues } => {
                    if queues.is_empty() {
                        println!("No queues found");
                        return Ok(());
                    }
                    match format {
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&queues)?);
                        }
                        _ => {
                            for q in &queues {
                                let workers_str = if q.workers.is_empty() {
                                    "-".to_string()
                                } else {
                                    q.workers.join(", ")
                                };
                                let ns_label = if q.namespace.is_empty() {
                                    "(no project)".to_string()
                                } else {
                                    q.namespace.clone()
                                };
                                println!(
                                    "{}\t{}\t{}\titems={}\tworkers={}",
                                    ns_label, q.name, q.queue_type, q.item_count, workers_str,
                                );
                            }
                        }
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
        QueueCommand::Items { queue, project } => {
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok())
                .unwrap_or_else(|| namespace.to_string());
            let request = Request::Query {
                query: Query::ListQueueItems {
                    queue_name: queue.clone(),
                    namespace: effective_namespace,
                },
            };
            match client.send(&request).await? {
                Response::QueueItems { mut items } => {
                    items.sort_by(|a, b| b.pushed_at_epoch_ms.cmp(&a.pushed_at_epoch_ms));
                    if items.is_empty() {
                        println!("No items in queue '{}'", queue);
                        return Ok(());
                    }
                    match format {
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&items)?);
                        }
                        _ => {
                            for item in &items {
                                let data_str: String = item
                                    .data
                                    .iter()
                                    .map(|(k, v)| format!("{}={}", k, v))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                let worker = item.worker_name.as_deref().unwrap_or("-");
                                println!(
                                    "{}\t{}\tworker={}\t{}",
                                    &item.id[..8],
                                    item.status,
                                    worker,
                                    data_str,
                                );
                            }
                        }
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
    }
    Ok(())
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
