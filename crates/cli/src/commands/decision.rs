// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Decision command handlers

use std::io::{BufRead, IsTerminal, Write};

use anyhow::Result;
use clap::{Args, Subcommand};

use oj_core::ShortId;
use oj_daemon::protocol::DecisionDetail;

use crate::client::{ClientKind, DaemonClient};
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
    /// Interactively review pending decisions
    Review {},
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

impl DecisionCommand {
    pub fn client_kind(&self) -> ClientKind {
        match self {
            Self::List {} | Self::Show { .. } => ClientKind::Query,
            Self::Resolve { .. } | Self::Review {} => ClientKind::Action,
        }
    }
}

pub async fn handle(
    command: DecisionCommand,
    client: &DaemonClient,
    namespace: &str,
    project_filter: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        DecisionCommand::List {} => {
            let mut decisions = client.list_decisions(namespace).await?;
            if let Some(proj) = project_filter {
                decisions.retain(|d| d.namespace == proj);
            }
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

        DecisionCommand::Show { id } => {
            let decision = client.get_decision(&id).await?;
            if let Some(d) = decision {
                match format {
                    OutputFormat::Json => {
                        println!("{}", serde_json::to_string_pretty(&d)?);
                    }
                    _ => {
                        format_decision_detail(&mut std::io::stdout(), &d, true);
                    }
                }
            } else {
                anyhow::bail!("decision not found: {}", id);
            }
        }

        DecisionCommand::Review {} => {
            if !std::io::stdin().is_terminal() {
                anyhow::bail!("review requires an interactive terminal");
            }
            if format == OutputFormat::Json {
                anyhow::bail!("review does not support --output json");
            }

            let mut decisions = client.list_decisions(namespace).await?;
            if let Some(proj) = project_filter {
                decisions.retain(|d| d.namespace == proj);
            }

            if decisions.is_empty() {
                println!("No pending decisions");
                return Ok(());
            }

            let total = decisions.len();
            println!(
                "{} pending decision{}",
                total,
                if total == 1 { "" } else { "s" }
            );
            println!();

            let mut resolved = 0usize;
            let mut skipped = 0usize;
            let stdin = std::io::stdin();
            let mut lines = stdin.lock().lines();

            for (i, summary) in decisions.iter().enumerate() {
                let detail = match client.get_decision(&summary.id).await {
                    Ok(Some(d)) if d.resolved_at_ms.is_none() => d,
                    Ok(_) => {
                        skipped += 1;
                        continue;
                    }
                    Err(e) => {
                        eprintln!("error fetching {}: {}", summary.id.short(8), e);
                        skipped += 1;
                        continue;
                    }
                };

                println!("[{}/{}]", i + 1, total);
                format_decision_detail(&mut std::io::stdout(), &detail, false);

                let option_count = detail.options.len();
                let prompt_label = if option_count > 0 {
                    format!("Choose [1-{}=pick, s=skip, q=quit]: ", option_count)
                } else {
                    "Choose [s=skip, q=quit]: ".to_string()
                };
                eprint!("{}", prompt_label);
                std::io::stderr().flush().ok();

                let line = match lines.next() {
                    Some(Ok(l)) => l,
                    _ => break,
                };

                match parse_review_input(&line, option_count) {
                    ReviewAction::Pick(n) => {
                        eprint!("Message (Enter to skip): ");
                        std::io::stderr().flush().ok();
                        let msg_line = match lines.next() {
                            Some(Ok(l)) => l,
                            _ => String::new(),
                        };
                        let message = if msg_line.trim().is_empty() {
                            None
                        } else {
                            Some(msg_line.trim().to_string())
                        };

                        match client.decision_resolve(&detail.id, Some(n), message).await {
                            Ok(_) => {
                                let label = detail
                                    .options
                                    .iter()
                                    .find(|o| o.number == n)
                                    .map(|o| o.label.as_str())
                                    .unwrap_or("?");
                                println!("  Resolved {} -> {} ({})", detail.id.short(8), n, label);
                                resolved += 1;
                            }
                            Err(e) => {
                                eprintln!("  error: {}", e);
                                skipped += 1;
                            }
                        }
                    }
                    ReviewAction::Skip => {
                        skipped += 1;
                    }
                    ReviewAction::Quit => {
                        skipped += total - i - resolved;
                        break;
                    }
                    ReviewAction::Invalid => {
                        eprintln!("  invalid input, skipping");
                        skipped += 1;
                    }
                }
                println!();
            }

            println!("Done. {} resolved, {} skipped.", resolved, skipped);
        }

        DecisionCommand::Resolve {
            id,
            choice,
            message,
        } => {
            let resolved_id = client.decision_resolve(&id, choice, message).await?;
            println!("Resolved decision {}", resolved_id.short(8));
        }
    }
    Ok(())
}

pub(crate) fn format_decision_detail(
    out: &mut impl Write,
    d: &DecisionDetail,
    show_resolve_hint: bool,
) {
    let short_id = d.id.short(8);
    let job_display = if d.job_name.is_empty() {
        d.job_id.clone()
    } else {
        format!("{} ({})", d.job_name, d.job_id.short(8))
    };
    let age = format_time_ago(d.created_at_ms);

    let _ = writeln!(
        out,
        "{} {}",
        color::header("Decision:"),
        color::muted(short_id)
    );
    let _ = writeln!(out, "{} {}", color::context("Job:"), job_display);
    let _ = writeln!(out, "{} {}", color::context("Source:  "), d.source);
    let _ = writeln!(out, "{} {}", color::context("Age:    "), age);
    if let Some(ref aid) = d.agent_id {
        let _ = writeln!(
            out,
            "{} {}",
            color::context("Agent:  "),
            color::muted(aid.short(8))
        );
    }

    if let Some(ref sup_id) = d.superseded_by {
        let _ = writeln!(
            out,
            "{} {}",
            color::context("Status: "),
            color::muted("superseded")
        );
        let _ = writeln!(
            out,
            "{} {}",
            color::context("Superseded by:"),
            color::muted(sup_id.short(8))
        );
    } else if d.resolved_at_ms.is_some() {
        let _ = writeln!(
            out,
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
            let _ = writeln!(out, "{} {} ({})", color::context("Chosen: "), c, label);
        }
        if let Some(ref m) = d.message {
            let _ = writeln!(out, "{} {}", color::context("Message:"), m);
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "{}", color::header("Context:"));
    for line in d.context.lines() {
        let _ = writeln!(out, "  {}", line);
    }

    if !d.options.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", color::header("Options:"));
        for opt in &d.options {
            let rec = if opt.recommended {
                " (recommended)"
            } else {
                ""
            };
            let _ = write!(out, "  {}. {}{}", opt.number, opt.label, rec);
            if let Some(ref desc) = opt.description {
                let _ = write!(out, " - {}", desc);
            }
            let _ = writeln!(out);
        }
    }

    if show_resolve_hint && d.resolved_at_ms.is_none() {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Use: oj decision resolve {} <number> [-m message]",
            short_id
        );
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum ReviewAction {
    Pick(usize),
    Skip,
    Quit,
    Invalid,
}

pub(crate) fn parse_review_input(input: &str, option_count: usize) -> ReviewAction {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed == "s" || trimmed == "S" {
        return ReviewAction::Skip;
    }
    if trimmed == "q" || trimmed == "Q" || trimmed == "x" || trimmed == "X" {
        return ReviewAction::Quit;
    }
    if let Ok(n) = trimmed.parse::<usize>() {
        if n >= 1 && n <= option_count {
            return ReviewAction::Pick(n);
        }
    }
    ReviewAction::Invalid
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
        Column::left("JOB").with_max(18),
        Column::left("AGE"),
        Column::left("SOURCE"),
        Column::left("SUMMARY").with_max(50),
    ]);
    let mut table = Table::new(cols);

    for d in decisions {
        let job = if d.job_name.is_empty() {
            &d.job_id
        } else {
            &d.job_name
        };
        let mut cells = vec![d.id.clone()];
        if show_project {
            cells.push(project_cell(&d.namespace));
        }
        cells.extend([
            job.to_string(),
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
