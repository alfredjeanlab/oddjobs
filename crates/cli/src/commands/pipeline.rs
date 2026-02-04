// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj pipeline` - Pipeline management commands

use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, Subcommand};

use oj_core::ShortId;

use crate::client::DaemonClient;
use crate::color;
use crate::output::{
    display_log, format_time_ago, print_prune_results, should_use_color, OutputFormat,
};
use crate::table::{project_cell, should_show_project, Column, Table};

#[derive(Args)]
pub struct PipelineArgs {
    #[command(subcommand)]
    pub command: PipelineCommand,
}

#[derive(Subcommand)]
pub enum PipelineCommand {
    /// List pipelines
    List {
        /// Filter by name substring
        name: Option<String>,

        /// Filter by status (e.g. "running", "failed", "completed")
        #[arg(long)]
        status: Option<String>,

        /// Maximum number of pipelines to show (default: 20)
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,

        /// Show all pipelines (no limit)
        #[arg(long, conflicts_with = "limit")]
        no_limit: bool,
    },
    /// Show details of a pipeline
    Show {
        /// Pipeline ID or name
        id: String,

        /// Show full variable values without truncation
        #[arg(long, short = 'v')]
        verbose: bool,
    },
    /// Resume monitoring for an escalated pipeline
    Resume {
        /// Pipeline ID or name
        id: String,

        /// Message for nudge/recovery (required for agent steps)
        #[arg(short = 'm', long)]
        message: Option<String>,

        /// Pipeline variables to set (can be repeated: --var key=value)
        #[arg(long = "var", value_parser = parse_key_value)]
        var: Vec<(String, String)>,
    },
    /// Cancel one or more running pipelines
    Cancel {
        /// Pipeline IDs or names (prefix match)
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Attach to the agent session for a pipeline
    Attach {
        /// Pipeline ID (supports prefix matching)
        id: String,
    },
    /// View pipeline activity logs
    Logs {
        /// Pipeline ID or name
        id: String,
        /// Stream live activity (like tail -f)
        #[arg(long, short)]
        follow: bool,
        /// Number of recent lines to show (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
    },
    /// Peek at the active tmux session for a pipeline
    Peek {
        /// Pipeline ID (supports prefix matching)
        id: String,
    },
    /// Remove old terminal pipelines (failed/cancelled/done)
    Prune {
        /// Remove all terminal pipelines regardless of age
        #[arg(long)]
        all: bool,
        /// Remove all failed pipelines regardless of age
        #[arg(long)]
        failed: bool,
        /// Prune orphaned pipelines (breadcrumb exists but no daemon state)
        #[arg(long)]
        orphans: bool,
        /// Show what would be pruned without doing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Block until pipeline(s) reach a terminal state
    Wait {
        /// Pipeline IDs or names (prefix match)
        #[arg(required = true)]
        ids: Vec<String>,

        /// Wait for ALL pipelines to complete (default: wait for ANY)
        #[arg(long)]
        all: bool,

        /// Timeout duration (e.g. "5m", "30s", "1h")
        #[arg(long)]
        timeout: Option<String>,
    },
}

/// Parse a key=value string for input arguments.
pub(crate) fn parse_key_value(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid input format '{}': must be key=value", s))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

/// Parse a human-readable duration string (e.g. "5m", "30s", "1h30m")
pub fn parse_duration(s: &str) -> Result<Duration> {
    let mut total_secs: u64 = 0;
    let mut current_num = String::new();

    for c in s.chars() {
        if c.is_ascii_digit() {
            current_num.push(c);
        } else {
            let n: u64 = current_num
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid duration: {}", s))?;
            current_num.clear();
            match c {
                'h' => total_secs += n * 3600,
                'm' => total_secs += n * 60,
                's' => total_secs += n,
                _ => anyhow::bail!("unknown duration unit '{}' in: {}", c, s),
            }
        }
    }
    // Bare number → seconds
    if !current_num.is_empty() {
        let n: u64 = current_num
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid duration: {}", s))?;
        total_secs += n;
    }
    if total_secs == 0 {
        anyhow::bail!("duration must be > 0: {}", s);
    }
    Ok(Duration::from_secs(total_secs))
}

pub(crate) fn format_pipeline_list(out: &mut impl Write, pipelines: &[oj_daemon::PipelineSummary]) {
    if pipelines.is_empty() {
        let _ = writeln!(out, "No pipelines");
        return;
    }

    // Show PROJECT column only when multiple namespaces present
    let show_project = should_show_project(pipelines.iter().map(|p| p.namespace.as_str()));

    // Show RETRIES column only when any pipeline has retries
    let show_retries = pipelines.iter().any(|p| p.retry_count > 0);

    // Build columns
    let mut cols = vec![Column::muted("ID")];
    if show_project {
        cols.push(Column::left("PROJECT"));
    }
    cols.extend([
        Column::left("NAME"),
        Column::left("KIND"),
        Column::left("STEP"),
        Column::left("UPDATED"),
    ]);
    if show_retries {
        cols.push(Column::left("RETRIES"));
    }
    cols.push(Column::status("STATUS"));

    let mut table = Table::new(cols);

    for p in pipelines {
        let id = p.id.short(8).to_string();
        let updated = format_time_ago(p.updated_at_ms);

        let mut cells = vec![id];
        if show_project {
            cells.push(project_cell(&p.namespace));
        }
        cells.extend([p.name.clone(), p.kind.clone(), p.step.clone(), updated]);
        if show_retries {
            cells.push(p.retry_count.to_string());
        }
        cells.push(p.step_status.clone());
        table.row(cells);
    }

    table.render(out);
}

pub async fn handle(
    command: PipelineCommand,
    client: &DaemonClient,
    project: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        PipelineCommand::List {
            name,
            status,
            limit,
            no_limit,
        } => {
            let mut pipelines = client.list_pipelines().await?;

            // Filter by explicit --project flag (OJ_NAMESPACE is NOT used for filtering)
            if let Some(proj) = project {
                pipelines.retain(|p| p.namespace == proj);
            }

            // Filter by name substring
            if let Some(ref pat) = name {
                let pat_lower = pat.to_lowercase();
                pipelines.retain(|p| p.name.to_lowercase().contains(&pat_lower));
            }

            // Filter by status
            if let Some(ref st) = status {
                let st_lower = st.to_lowercase();
                pipelines.retain(|p| {
                    p.step_status.to_lowercase() == st_lower || p.step.to_lowercase() == st_lower
                });
            }

            // Sort by most recently updated first
            pipelines.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));

            // Limit
            let total = pipelines.len();
            let effective_limit = if no_limit { total } else { limit };
            let truncated = total > effective_limit;
            if truncated {
                pipelines.truncate(effective_limit);
            }

            match format {
                OutputFormat::Text => {
                    let mut out = std::io::stdout();
                    format_pipeline_list(&mut out, &pipelines);

                    if truncated {
                        let remaining = total - effective_limit;
                        println!(
                            "\n... {} more not shown. Use --no-limit or --limit N to see more.",
                            remaining
                        );
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&pipelines)?);
                }
            }
        }
        PipelineCommand::Show { id, verbose } => {
            let pipeline = client.get_pipeline(&id).await?;

            match format {
                OutputFormat::Text => {
                    if let Some(p) = pipeline {
                        println!("{} {}", color::header("Pipeline:"), p.id);
                        println!("  {} {}", color::context("Name:"), p.name);
                        if !p.namespace.is_empty() {
                            println!("  {} {}", color::context("Project:"), p.namespace);
                        }
                        println!("  {} {}", color::context("Kind:"), p.kind);
                        println!(
                            "  {} {}",
                            color::context("Status:"),
                            color::status(&p.step_status)
                        );
                        if let Some(session) = &p.session_id {
                            println!("  {} {}", color::context("Session:"), session);
                        }
                        if let Some(ws) = &p.workspace_path {
                            println!("  {} {}", color::context("Workspace:"), ws.display());
                        }
                        if let Some(error) = &p.error {
                            println!();
                            println!("  {} {}", color::context("Error:"), error);
                        }

                        if !p.steps.is_empty() {
                            println!();
                            println!("  {}", color::header("Steps:"));
                            for step in &p.steps {
                                let duration = super::pipeline_wait::format_duration(
                                    step.started_at_ms,
                                    step.finished_at_ms,
                                );
                                let status = match step.outcome.as_str() {
                                    "completed" => "completed".to_string(),
                                    "running" => "running".to_string(),
                                    "failed" => match &step.detail {
                                        Some(d) => {
                                            format!("failed ({})", truncate(d, 40))
                                        }
                                        None => "failed".to_string(),
                                    },
                                    "waiting" => match &step.detail {
                                        Some(d) => {
                                            format!("waiting ({})", truncate(d, 40))
                                        }
                                        None => "waiting".to_string(),
                                    },
                                    other => other.to_string(),
                                };
                                println!(
                                    "    {:<12} {:<8} {}",
                                    step.name,
                                    duration,
                                    color::status(&status)
                                );
                            }
                        }

                        if !p.agents.is_empty() {
                            println!();
                            println!("  {}", color::header("Agents:"));
                            for agent in &p.agents {
                                let summary = format_agent_summary(agent);
                                let session_id = truncate(&agent.agent_id, 8);
                                if summary.is_empty() {
                                    println!(
                                        "    {:<12} {} {}",
                                        agent.step_name,
                                        color::status(&format!("{:<12}", &agent.status)),
                                        color::muted(session_id),
                                    );
                                } else {
                                    println!(
                                        "    {:<12} {} {} ({})",
                                        agent.step_name,
                                        color::status(&format!("{:<12}", &agent.status)),
                                        summary,
                                        color::muted(session_id),
                                    );
                                }
                            }
                        }

                        if !p.vars.is_empty() {
                            println!("  {}", color::header("Variables:"));
                            if verbose {
                                for (k, v) in &p.vars {
                                    if v.contains('\n') {
                                        println!("    {}:", k);
                                        for line in v.lines() {
                                            println!("      {}", line);
                                        }
                                    } else {
                                        println!("    {}: {}", k, v);
                                    }
                                }
                            } else {
                                for (k, v) in &p.vars {
                                    println!("    {}: {}", k, format_var_value(v, 80));
                                }
                                let any_truncated =
                                    p.vars.values().any(|v| is_var_truncated(v, 80));
                                if any_truncated {
                                    println!(
                                        "  {}",
                                        color::muted("hint: use --verbose to show full variables")
                                    );
                                }
                            }
                        }
                    } else {
                        println!("Pipeline not found: {}", id);
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&pipeline)?);
                }
            }
        }
        PipelineCommand::Resume { id, message, var } => {
            let var_map: HashMap<String, String> = var.into_iter().collect();
            match client
                .pipeline_resume(&id, message.as_deref(), &var_map)
                .await
            {
                Ok(()) => {
                    if !var_map.is_empty() {
                        println!("Updated vars and resumed pipeline {}", id);
                    } else {
                        println!("Resumed pipeline {}", id);
                    }
                }
                Err(crate::client::ClientError::Rejected(msg))
                    if msg.contains("--message") || msg.contains("agent steps require") =>
                {
                    eprintln!("error: {}", msg);
                    std::process::exit(1);
                }
                Err(e) => return Err(e.into()),
            }
        }
        PipelineCommand::Cancel { ids } => {
            let result = client.pipeline_cancel(&ids).await?;

            for id in &result.cancelled {
                println!("Cancelled pipeline {}", id);
            }
            for id in &result.already_terminal {
                println!("Pipeline {} was already terminal", id);
            }
            for id in &result.not_found {
                eprintln!("Pipeline not found: {}", id);
            }

            // Exit with error if any pipelines were not found
            if !result.not_found.is_empty() {
                std::process::exit(1);
            }
        }
        PipelineCommand::Attach { id } => {
            let pipeline = client
                .get_pipeline(&id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("pipeline not found: {}", id))?;
            let session_id = pipeline
                .session_id
                .ok_or_else(|| anyhow::anyhow!("pipeline has no active session"))?;
            super::session::attach(&session_id)?;
        }
        PipelineCommand::Peek { id } => {
            let pipeline = client
                .get_pipeline(&id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Pipeline not found: {}", id))?;

            // Try main pipeline session first, then fall back to running agent session
            let session_id = pipeline.session_id.clone().or_else(|| {
                pipeline
                    .agents
                    .iter()
                    .find(|a| a.status == "running")
                    .map(|a| a.agent_id.clone())
            });

            // Try to capture the tmux pane output if a session exists
            let peek_output = if let Some(ref session_id) = session_id {
                let with_color = should_use_color();
                match client.peek_session(session_id, with_color).await {
                    Ok(output) => Some((session_id.clone(), output)),
                    Err(crate::client::ClientError::Rejected(msg))
                        if msg.starts_with("Session not found") =>
                    {
                        None
                    }
                    Err(e) => return Err(e.into()),
                }
            } else {
                None
            };

            match peek_output {
                Some((session_id, output)) => {
                    println!(
                        "{}",
                        color::header(&format!("╭──── peek: {} ────", session_id))
                    );
                    print!("{}", output);
                    println!("{}", color::header("╰──── end peek ────"));
                }
                None => {
                    let short_id = pipeline.id.short(8);
                    let is_terminal = pipeline.step == "done"
                        || pipeline.step == "failed"
                        || pipeline.step == "cancelled";

                    if is_terminal {
                        println!(
                            "Pipeline {} is {}. No active session.",
                            short_id, pipeline.step
                        );
                    } else {
                        println!(
                            "No active session for pipeline {} (step: {}, status: {})",
                            short_id, pipeline.step, pipeline.step_status
                        );
                    }
                    println!();
                    println!("Try:");
                    println!("    oj pipeline logs {}", short_id);
                    println!("    oj pipeline show {}", short_id);
                }
            }
        }
        PipelineCommand::Logs { id, follow, limit } => {
            let (log_path, content) = client.get_pipeline_logs(&id, limit).await?;
            display_log(&log_path, &content, follow, format, "pipeline", &id).await?;
        }
        PipelineCommand::Prune {
            all,
            failed,
            orphans,
            dry_run,
        } => {
            // Only scope by namespace when explicitly requested via --project.
            // Without this, prune matches `pipeline list` behavior and operates
            // across all namespaces — fixing the bug where auto-resolved namespace
            // silently skipped pipelines from other projects.
            let (pruned, skipped) = client
                .pipeline_prune(all, failed, orphans, dry_run, project)
                .await?;

            print_prune_results(
                &pruned,
                skipped,
                dry_run,
                format,
                "pipeline",
                "skipped",
                |entry| {
                    let short_id = entry.id.short(8);
                    format!("{} ({}, {})", entry.name, short_id, entry.step)
                },
            )?;
        }
        PipelineCommand::Wait { ids, all, timeout } => {
            super::pipeline_wait::handle(ids, all, timeout, client).await?;
        }
    }

    Ok(())
}

/// Print follow-up commands for a pipeline.
pub(crate) fn print_pipeline_commands(short_id: &str) {
    println!("    oj pipeline show {short_id}");
    println!(
        "    oj pipeline wait {short_id}      {}",
        color::muted("# Wait until pipeline ends")
    );
    println!(
        "    oj pipeline logs {short_id} -f   {}",
        color::muted("# Follow logs")
    );
    println!(
        "    oj pipeline peek {short_id}      {}",
        color::muted("# Capture tmux pane")
    );
    println!(
        "    oj pipeline attach {short_id}    {}",
        color::muted("# Attach to tmux")
    );
}

fn format_agent_summary(agent: &oj_daemon::AgentSummary) -> String {
    let mut parts = Vec::new();
    if agent.files_read > 0 {
        parts.push(format!(
            "{} file{} read",
            agent.files_read,
            if agent.files_read == 1 { "" } else { "s" }
        ));
    }
    if agent.files_written > 0 {
        parts.push(format!(
            "{} file{} written",
            agent.files_written,
            if agent.files_written == 1 { "" } else { "s" }
        ));
    }
    if agent.commands_run > 0 {
        parts.push(format!(
            "{} command{}",
            agent.commands_run,
            if agent.commands_run == 1 { "" } else { "s" }
        ));
    }
    if let Some(ref reason) = agent.exit_reason {
        parts.push(format!("exit: {}", reason));
    }
    parts.join(", ")
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

fn format_var_value(value: &str, max_len: usize) -> String {
    let escaped = value.replace('\n', "\\n");
    if escaped.chars().count() <= max_len {
        escaped
    } else {
        let truncated: String = escaped.chars().take(max_len).collect();
        format!("{}...", truncated)
    }
}

fn is_var_truncated(value: &str, max_len: usize) -> bool {
    let escaped = value.replace('\n', "\\n");
    escaped.chars().count() > max_len
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
