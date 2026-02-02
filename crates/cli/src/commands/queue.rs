// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue command handlers

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::Path;

use oj_daemon::{Query, Request, Response};

use crate::client::DaemonClient;
use crate::output::OutputFormat;

#[derive(Args)]
pub struct QueueArgs {
    #[command(subcommand)]
    pub command: QueueCommand,
}

#[derive(Subcommand)]
pub enum QueueCommand {
    /// Push an item to a persisted queue
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
    /// List items in a persisted queue
    List {
        /// Queue name
        #[arg(long)]
        queue: String,
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
            let json_data = build_data_map(data, var)?;

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
        QueueCommand::List { queue } => {
            let request = Request::Query {
                query: Query::ListQueueItems {
                    queue_name: queue.clone(),
                },
            };
            match client.send(&request).await? {
                Response::QueueItems { items } => {
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
