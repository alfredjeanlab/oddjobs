// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Queue command handlers

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::Path;

use oj_core::ShortId;

use crate::client::{ClientKind, DaemonClient, QueuePushResult, QueueRetryResult};
use crate::color;
use crate::output::{display_log, format_time_ago, print_prune_results, OutputFormat};
use crate::table::{project_cell, should_show_project, Column, Table};

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
        #[arg(long = "var", value_parser = super::job::parse_key_value)]
        var: Vec<(String, String)>,
    },
    /// List all known queues
    List {},
    /// Show items in a specific queue
    Show {
        /// Queue name
        queue: String,
    },
    /// Remove an item from a persisted queue
    Drop {
        /// Queue name
        queue: String,
        /// Item ID (or prefix)
        item_id: String,
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
    },
    /// Retry dead or failed queue items
    Retry {
        /// Queue name
        queue: String,
        /// Item IDs (or prefixes) to retry
        #[arg(required_unless_present_any = ["all_dead", "status"])]
        item_ids: Vec<String>,
        /// Retry all dead items
        #[arg(long)]
        all_dead: bool,
        /// Retry items with specific status (dead or failed)
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
    },
    /// Mark an active queue item as failed
    Fail {
        /// Queue name
        queue: String,
        /// Item ID (or prefix)
        item_id: String,
    },
    /// Mark an active queue item as completed
    Done {
        /// Queue name
        queue: String,
        /// Item ID (or prefix)
        item_id: String,
    },
    /// Remove and return all pending items from a persisted queue
    Drain {
        /// Queue name
        queue: String,
    },
    /// Remove completed and dead items from a queue
    Prune {
        /// Queue name
        queue: String,
        /// Prune all terminal items regardless of age
        #[arg(long)]
        all: bool,
        /// Show what would be pruned without making changes
        #[arg(long)]
        dry_run: bool,
    },
}

impl QueueCommand {
    pub fn client_kind(&self) -> ClientKind {
        match self {
            Self::List {} | Self::Show { .. } | Self::Logs { .. } => ClientKind::Query,
            _ => ClientKind::Action,
        }
    }
}

/// Format a queue item's data map as a sorted `key=value` string.
fn format_item_data(data: &std::collections::HashMap<String, String>) -> String {
    let mut pairs: Vec<_> = data.iter().collect();
    pairs.sort_by_key(|(k, _)| k.as_str());
    pairs
        .into_iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(" ")
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
        QueueCommand::Push { queue, data, var } => {
            // Build data map; allow empty data for external queues (triggers poll)
            let json_data = if data.is_none() && var.is_empty() {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                build_data_map(data, var)?
            };

            match client
                .queue_push(project_root, namespace, &queue, json_data)
                .await?
            {
                QueuePushResult::Pushed {
                    queue_name,
                    item_id,
                } => {
                    println!("Pushed item '{}' to queue '{}'", item_id, queue_name);
                }
                QueuePushResult::Refreshed => {
                    println!("Refreshed external queue '{}'", queue);
                }
            }
        }
        QueueCommand::Drop { queue, item_id } => {
            let (queue_name, item_id) = client
                .queue_drop(project_root, namespace, &queue, &item_id)
                .await?;
            println!(
                "Dropped item {} from queue {}",
                item_id.short(8),
                queue_name
            );
        }
        QueueCommand::Retry {
            queue,
            item_ids,
            all_dead,
            status,
        } => {
            // Validate --status if provided
            if let Some(ref s) = status {
                let s_lower = s.to_lowercase();
                if s_lower != "dead" && s_lower != "failed" {
                    anyhow::bail!("invalid status '{}': must be 'dead' or 'failed'", s);
                }
            }

            match client
                .queue_retry(project_root, namespace, &queue, item_ids, all_dead, status)
                .await?
            {
                QueueRetryResult::Single {
                    queue_name,
                    item_id,
                } => {
                    println!("Retrying item {} in queue {}", item_id.short(8), queue_name);
                }
                QueueRetryResult::Bulk {
                    queue_name,
                    item_ids: retried_ids,
                    already_retried,
                    not_found,
                } => {
                    if !retried_ids.is_empty() {
                        println!(
                            "Retried {} item{} in queue '{}'",
                            retried_ids.len(),
                            if retried_ids.len() == 1 { "" } else { "s" },
                            queue_name
                        );
                        for id in &retried_ids {
                            println!("  {}", id.short(8));
                        }
                    }
                    if !already_retried.is_empty() {
                        println!(
                            "Skipped {} item{} (not dead/failed):",
                            already_retried.len(),
                            if already_retried.len() == 1 { "" } else { "s" }
                        );
                        for id in &already_retried {
                            println!("  {}", id.short(8));
                        }
                    }
                    if !not_found.is_empty() {
                        println!("Not found: {}", not_found.join(", "));
                    }
                    if retried_ids.is_empty() && already_retried.is_empty() && not_found.is_empty()
                    {
                        println!("No items to retry in queue '{}'", queue_name);
                    }
                }
            }
        }
        QueueCommand::Fail { queue, item_id } => {
            let (queue_name, item_id) = client
                .queue_fail(project_root, namespace, &queue, &item_id)
                .await?;
            println!("Failed item {} in queue {}", item_id.short(8), queue_name);
        }
        QueueCommand::Done { queue, item_id } => {
            let (queue_name, item_id) = client
                .queue_done(project_root, namespace, &queue, &item_id)
                .await?;
            println!(
                "Completed item {} in queue {}",
                item_id.short(8),
                queue_name
            );
        }
        QueueCommand::Drain { queue } => {
            let (queue_name, items) = client.queue_drain(project_root, namespace, &queue).await?;
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&items)?);
                }
                _ => {
                    if items.is_empty() {
                        println!("No pending items in queue '{}'", queue_name);
                    } else {
                        println!(
                            "Drained {} item{} from queue '{}'",
                            items.len(),
                            if items.len() == 1 { "" } else { "s" },
                            queue_name
                        );
                        for item in &items {
                            let data_str = format_item_data(&item.data);
                            println!("  {} {}", color::muted(item.id.short(8)), data_str,);
                        }
                    }
                }
            }
        }
        QueueCommand::Logs {
            queue,
            follow,
            limit,
        } => {
            let (log_path, content) = client.get_queue_logs(&queue, namespace, limit).await?;
            display_log(&log_path, &content, follow, format, "queue", &queue).await?;
        }
        QueueCommand::List {} => {
            let queues = client.list_queues(project_root, namespace).await?;
            if queues.is_empty() {
                println!("No queues found");
                return Ok(());
            }
            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&queues)?);
                }
                _ => {
                    let show_project =
                        should_show_project(queues.iter().map(|q| q.namespace.as_str()));

                    let mut cols = Vec::new();
                    if show_project {
                        cols.push(Column::left("PROJECT"));
                    }
                    cols.extend([
                        Column::left("NAME"),
                        Column::left("TYPE"),
                        Column::right("ITEMS"),
                        Column::right("POLL"),
                        Column::muted("POLLED"),
                        Column::left("WORKERS"),
                    ]);
                    let mut table = Table::new(cols);

                    for q in &queues {
                        let workers_str = if q.workers.is_empty() {
                            "-".to_string()
                        } else {
                            q.workers.join(", ")
                        };
                        let poll_count = q
                            .last_poll_count
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "-".to_string());
                        let polled_at = q
                            .last_polled_at_ms
                            .map(format_time_ago)
                            .unwrap_or_else(|| "-".to_string());
                        let mut cells = Vec::new();
                        if show_project {
                            cells.push(project_cell(&q.namespace));
                        }
                        cells.extend([
                            q.name.clone(),
                            q.queue_type.clone(),
                            q.item_count.to_string(),
                            poll_count,
                            polled_at,
                            workers_str,
                        ]);
                        table.row(cells);
                    }
                    table.render(&mut std::io::stdout());
                }
            }
        }
        QueueCommand::Prune {
            queue,
            all,
            dry_run,
        } => {
            let (pruned, skipped) = client
                .queue_prune(project_root, namespace, &queue, all, dry_run)
                .await?;

            print_prune_results(
                &pruned,
                skipped,
                dry_run,
                format,
                "queue item",
                "skipped",
                |entry| {
                    format!(
                        "item {} ({}) from queue '{}'",
                        &entry.item_id[..8.min(entry.item_id.len())],
                        entry.status,
                        entry.queue_name,
                    )
                },
            )?;
        }
        QueueCommand::Show { queue } => {
            let mut items = client
                .list_queue_items(&queue, namespace, Some(project_root))
                .await?;
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
                    let mut table = Table::new(vec![
                        Column::muted("ID"),
                        Column::status("STATUS"),
                        Column::right("AGE"),
                        Column::left("WORKER"),
                        Column::left("DATA"),
                    ]);
                    for item in &items {
                        let data_str = format_item_data(&item.data);
                        let worker = item.worker_name.as_deref().unwrap_or("-").to_string();
                        let age = format_time_ago(item.pushed_at_epoch_ms);
                        table.row(vec![
                            item.id.short(8).to_string(),
                            item.status.clone(),
                            age,
                            worker,
                            data_str,
                        ]);
                    }
                    table.render(&mut std::io::stdout());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
