// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision command handlers

use anyhow::Result;
use clap::{Args, Subcommand};

use oj_daemon::{Query, Request, Response};

use crate::client::DaemonClient;
use crate::color;
use crate::output::{format_time_ago, OutputFormat};

#[derive(Args)]
pub struct DecisionArgs {
    #[command(subcommand)]
    pub command: DecisionCommand,
}

#[derive(Subcommand)]
pub enum DecisionCommand {
    /// List pending decisions
    List {
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
    /// Show details of a decision
    Show {
        /// Decision ID (or prefix)
        id: String,
    },
    /// Resolve a pending decision
    Resolve {
        /// Decision ID (or prefix)
        id: String,
        /// Pick a numbered option (1-indexed)
        choice: Option<usize>,
        /// Freeform message or answer
        #[arg(short = 'm', long)]
        message: Option<String>,
    },
}

pub async fn handle(
    command: DecisionCommand,
    client: &DaemonClient,
    namespace: &str,
    format: OutputFormat,
) -> Result<()> {
    match command {
        DecisionCommand::List { project } => {
            // Namespace resolution: --project flag > OJ_NAMESPACE env > resolved namespace
            // (empty OJ_NAMESPACE treated as unset)
            let effective_namespace = project
                .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| namespace.to_string());
            let request = Request::Query {
                query: Query::ListDecisions {
                    namespace: effective_namespace,
                },
            };
            match client.send(&request).await? {
                Response::Decisions { decisions } => {
                    if decisions.is_empty() {
                        println!("No pending decisions");
                        return Ok(());
                    }
                    match format {
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&decisions)?);
                        }
                        _ => {
                            println!(
                                "{} {} {} {} {}",
                                color::header(&format!("{:<10}", "ID")),
                                color::header(&format!("{:<20}", "PIPELINE")),
                                color::header(&format!("{:<8}", "AGE")),
                                color::header(&format!("{:<8}", "SOURCE")),
                                color::header("SUMMARY"),
                            );
                            for d in &decisions {
                                let short_id = if d.id.len() > 8 { &d.id[..8] } else { &d.id };
                                let age = format_time_ago(d.created_at_ms);
                                let pipeline = if d.pipeline_name.is_empty() {
                                    &d.pipeline_id
                                } else {
                                    &d.pipeline_name
                                };
                                let pipeline_display = if pipeline.len() > 18 {
                                    format!("{}...", &pipeline[..15])
                                } else {
                                    pipeline.to_string()
                                };
                                let summary = if d.summary.len() > 50 {
                                    format!("{}...", &d.summary[..47])
                                } else {
                                    d.summary.clone()
                                };
                                println!(
                                    "{} {:<20} {:<8} {:<8} {}",
                                    color::muted(&format!("{:<10}", short_id)),
                                    pipeline_display,
                                    age,
                                    d.source,
                                    summary
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

        DecisionCommand::Show { id } => {
            let request = Request::Query {
                query: Query::GetDecision { id: id.clone() },
            };
            match client.send(&request).await? {
                Response::Decision { decision } => {
                    if let Some(d) = decision {
                        match format {
                            OutputFormat::Json => {
                                println!("{}", serde_json::to_string_pretty(&*d)?);
                            }
                            _ => {
                                let short_id = if d.id.len() > 8 { &d.id[..8] } else { &d.id };
                                let pipeline_display = if d.pipeline_name.is_empty() {
                                    d.pipeline_id.clone()
                                } else {
                                    format!(
                                        "{} ({})",
                                        d.pipeline_name,
                                        &d.pipeline_id[..8.min(d.pipeline_id.len())]
                                    )
                                };
                                let age = format_time_ago(d.created_at_ms);

                                println!(
                                    "{} {}",
                                    color::header("Decision:"),
                                    color::muted(short_id)
                                );
                                println!("{} {}", color::context("Pipeline:"), pipeline_display);
                                println!("{} {}", color::context("Source:  "), d.source);
                                println!("{} {}", color::context("Age:    "), age);
                                if let Some(ref aid) = d.agent_id {
                                    println!(
                                        "{} {}",
                                        color::context("Agent:  "),
                                        color::muted(&aid[..8.min(aid.len())])
                                    );
                                }

                                if d.resolved_at_ms.is_some() {
                                    println!(
                                        "{} {}",
                                        color::context("Status: "),
                                        color::status("completed")
                                    );
                                    if let Some(c) = d.chosen {
                                        let label = d
                                            .options
                                            .iter()
                                            .find(|o| o.number == c)
                                            .map(|o| o.label.as_str())
                                            .unwrap_or("?");
                                        println!(
                                            "{} {} ({})",
                                            color::context("Chosen: "),
                                            c,
                                            label
                                        );
                                    }
                                    if let Some(ref m) = d.message {
                                        println!("{} {}", color::context("Message:"), m);
                                    }
                                }

                                println!();
                                println!("{}", color::header("Context:"));
                                for line in d.context.lines() {
                                    println!("  {}", line);
                                }

                                if !d.options.is_empty() {
                                    println!();
                                    println!("{}", color::header("Options:"));
                                    for opt in &d.options {
                                        let rec = if opt.recommended {
                                            " (recommended)"
                                        } else {
                                            ""
                                        };
                                        print!("  {}. {}{}", opt.number, opt.label, rec);
                                        if let Some(ref desc) = opt.description {
                                            print!(" - {}", desc);
                                        }
                                        println!();
                                    }
                                }

                                if d.resolved_at_ms.is_none() {
                                    println!();
                                    println!(
                                        "Use: oj decision resolve {} <number> [-m message]",
                                        short_id
                                    );
                                }
                            }
                        }
                    } else {
                        anyhow::bail!("decision not found: {}", id);
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

        DecisionCommand::Resolve {
            id,
            choice,
            message,
        } => {
            let request = Request::DecisionResolve {
                id: id.clone(),
                chosen: choice,
                message,
            };
            match client.send(&request).await? {
                Response::DecisionResolved { id: resolved_id } => {
                    let short_id = if resolved_id.len() > 8 {
                        &resolved_id[..8]
                    } else {
                        &resolved_id
                    };
                    println!("Resolved decision {}", short_id);
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
#[path = "decision_tests.rs"]
mod tests;
