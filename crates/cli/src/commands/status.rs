// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Status command handler

use anyhow::Result;

use crate::client::DaemonClient;
use crate::output::OutputFormat;

pub async fn handle(format: OutputFormat) -> Result<()> {
    let client = match DaemonClient::connect() {
        Ok(c) => c,
        Err(_) => {
            println!("Daemon is not running");
            return Ok(());
        }
    };

    let (uptime_secs, namespaces) = client.status_overview().await?;

    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "uptime_secs": uptime_secs,
                    "namespaces": namespaces,
                }))?
            );
        }
        _ => {
            println!("Daemon uptime: {}s", uptime_secs);
            if namespaces.is_empty() {
                println!("No active work");
            }
            for ns in &namespaces {
                let label = if ns.namespace.is_empty() {
                    "(default)".to_string()
                } else {
                    ns.namespace.clone()
                };
                println!("\n[{}]", label);
                if !ns.active_pipelines.is_empty() {
                    println!("  Active pipelines: {}", ns.active_pipelines.len());
                }
                if !ns.escalated_pipelines.is_empty() {
                    println!("  Escalated: {}", ns.escalated_pipelines.len());
                }
                if !ns.workers.is_empty() {
                    println!("  Workers: {}", ns.workers.len());
                }
                if !ns.queues.is_empty() {
                    for q in &ns.queues {
                        println!(
                            "  Queue {}: pending={} active={} dead={}",
                            q.name, q.pending, q.active, q.dead
                        );
                    }
                }
            }
        }
    }

    Ok(())
}
