// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision command handlers

use std::io::Write;

use anyhow::Result;
use clap::{Args, Subcommand};

use oj_daemon::{Query, Request, Response};

use crate::client::DaemonClient;
use crate::color;
use crate::output::{format_time_ago, OutputFormat};
use crate::table::{project_cell, should_show_project, Column, Table};

#[derive(Args)]
pub struct DecisionArgs {
    #[command(subcommand)]
    pub command: DecisionCommand,
}

#[derive(Subcommand)]
pub enum DecisionCommand {
    /// List pending decisions
    List {},
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
        DecisionCommand::List {} => {
            let request = Request::Query {
                query: Query::ListDecisions {
                    namespace: namespace.to_string(),
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
                            format_decision_list(&mut std::io::stdout(), &decisions);
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

pub(crate) fn format_decision_list(
    out: &mut impl Write,
    decisions: &[oj_daemon::protocol::DecisionSummary],
) {
    let show_project = should_show_project(decisions.iter().map(|d| d.namespace.as_str()));

    let mut cols = vec![Column::muted("ID").with_max(8)];
    if show_project {
        cols.push(Column::left("PROJECT"));
    }
    cols.extend([
        Column::left("PIPELINE").with_max(18),
        Column::left("AGE"),
        Column::left("SOURCE"),
        Column::left("SUMMARY").with_max(50),
    ]);
    let mut table = Table::new(cols);

    for d in decisions {
        let pipeline = if d.pipeline_name.is_empty() {
            &d.pipeline_id
        } else {
            &d.pipeline_name
        };
        let mut cells = vec![d.id.clone()];
        if show_project {
            cells.push(project_cell(&d.namespace));
        }
        cells.extend([
            pipeline.to_string(),
            format_time_ago(d.created_at_ms),
            d.source.clone(),
            d.summary.clone(),
        ]);
        table.row(cells);
    }

    table.render(out);
}

#[cfg(test)]
#[path = "decision_tests.rs"]
mod tests;
