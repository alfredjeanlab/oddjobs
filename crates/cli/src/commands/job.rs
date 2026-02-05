// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj job` - Job management commands

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
pub struct JobArgs {
    #[command(subcommand)]
    pub command: JobCommand,
}

#[derive(Subcommand)]
pub enum JobCommand {
    /// List jobs
    List {
        /// Filter by name substring
        name: Option<String>,

        /// Filter by status (e.g. "running", "failed", "completed")
        #[arg(long)]
        status: Option<String>,

        /// Maximum number of jobs to show (default: 20)
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,

        /// Show all jobs (no limit)
        #[arg(long, conflicts_with = "limit")]
        no_limit: bool,
    },
    /// Show details of a job
    Show {
        /// Job ID or name
        id: String,

        /// Show full variable values without truncation
        #[arg(long, short = 'v')]
        verbose: bool,
    },
    /// Resume monitoring for an escalated job
    Resume {
        /// Job ID or name
        id: String,

        /// Message for nudge/recovery (required for agent steps)
        #[arg(short = 'm', long)]
        message: Option<String>,

        /// Job variables to set (can be repeated: --var key=value)
        #[arg(long = "var", value_parser = parse_key_value)]
        var: Vec<(String, String)>,
    },
    /// Cancel one or more running jobs
    Cancel {
        /// Job IDs or names (prefix match)
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Attach to the agent session for a job
    Attach {
        /// Job ID (supports prefix matching)
        id: String,
    },
    /// View job activity logs
    Logs {
        /// Job ID or name
        id: String,
        /// Stream live activity (like tail -f)
        #[arg(long, short)]
        follow: bool,
        /// Number of recent lines to show (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
    },
    /// Peek at the active tmux session for a job
    Peek {
        /// Job ID (supports prefix matching)
        id: String,
    },
    /// Remove old terminal jobs (failed/cancelled/done)
    Prune {
        /// Remove all terminal jobs regardless of age
        #[arg(long)]
        all: bool,
        /// Remove all failed jobs regardless of age
        #[arg(long)]
        failed: bool,
        /// Prune orphaned jobs (breadcrumb exists but no daemon state)
        #[arg(long)]
        orphans: bool,
        /// Show what would be pruned without doing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Block until job(s) reach a terminal state
    Wait {
        /// Job IDs or names (prefix match)
        #[arg(required = true)]
        ids: Vec<String>,

        /// Wait for ALL jobs to complete (default: wait for ANY)
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

pub(crate) fn format_job_list(out: &mut impl Write, jobs: &[oj_daemon::JobSummary]) {
    if jobs.is_empty() {
        let _ = writeln!(out, "No jobs");
        return;
    }

    // Show PROJECT column only when multiple namespaces present
    let show_project = should_show_project(jobs.iter().map(|p| p.namespace.as_str()));

    // Show RETRIES column only when any job has retries
    let show_retries = jobs.iter().any(|p| p.retry_count > 0);

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

    for p in jobs {
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
    command: JobCommand,
    client: &DaemonClient,
    project: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        JobCommand::List {
            name,
            status,
            limit,
            no_limit,
        } => {
            let mut jobs = client.list_jobs().await?;

            // Filter by explicit --project flag (OJ_NAMESPACE is NOT used for filtering)
            if let Some(proj) = project {
                jobs.retain(|p| p.namespace == proj);
            }

            // Filter by name substring
            if let Some(ref pat) = name {
                let pat_lower = pat.to_lowercase();
                jobs.retain(|p| p.name.to_lowercase().contains(&pat_lower));
            }

            // Filter by status
            if let Some(ref st) = status {
                let st_lower = st.to_lowercase();
                jobs.retain(|p| {
                    p.step_status.to_lowercase() == st_lower || p.step.to_lowercase() == st_lower
                });
            }

            // Sort by most recently updated first
            jobs.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));

            // Limit
            let total = jobs.len();
            let effective_limit = if no_limit { total } else { limit };
            let truncated = total > effective_limit;
            if truncated {
                jobs.truncate(effective_limit);
            }

            match format {
                OutputFormat::Text => {
                    let mut out = std::io::stdout();
                    format_job_list(&mut out, &jobs);

                    if truncated {
                        let remaining = total - effective_limit;
                        println!(
                            "\n... {} more not shown. Use --no-limit or --limit N to see more.",
                            remaining
                        );
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&jobs)?);
                }
            }
        }
        JobCommand::Show { id, verbose } => {
            let job = client.get_job(&id).await?;

            match format {
                OutputFormat::Text => {
                    if let Some(p) = job {
                        println!("{} {}", color::header("Job:"), p.id);
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
                                let duration = super::job_wait::format_duration(
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
                            println!();
                            println!("  {}", color::header("Variables:"));
                            let sorted = sorted_vars(&p.vars);
                            if verbose {
                                for (k, v) in &sorted {
                                    if v.contains('\n') {
                                        println!("    {}", color::context(&format!("{}:", k)));
                                        for line in v.lines() {
                                            println!("      {}", line);
                                        }
                                    } else {
                                        println!(
                                            "    {} {}",
                                            color::context(&format!("{}:", k)),
                                            v
                                        );
                                    }
                                }
                            } else {
                                for (k, v) in &sorted {
                                    println!(
                                        "    {} {}",
                                        color::context(&format!("{}:", k)),
                                        format_var_value(v, 80)
                                    );
                                }
                                let any_truncated =
                                    p.vars.values().any(|v| is_var_truncated(v, 80));
                                if any_truncated {
                                    println!();
                                    println!(
                                        "  {}",
                                        color::context(
                                            "hint: use --verbose to show full variables"
                                        )
                                    );
                                }
                            }
                        }
                    } else {
                        println!("Job not found: {}", id);
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&job)?);
                }
            }
        }
        JobCommand::Resume { id, message, var } => {
            let var_map: HashMap<String, String> = var.into_iter().collect();
            match client.job_resume(&id, message.as_deref(), &var_map).await {
                Ok(()) => {
                    if !var_map.is_empty() {
                        println!("Updated vars and resumed job {}", id);
                    } else {
                        println!("Resumed job {}", id);
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
        JobCommand::Cancel { ids } => {
            let result = client.job_cancel(&ids).await?;

            for id in &result.cancelled {
                println!("Cancelled job {}", id);
            }
            for id in &result.already_terminal {
                println!("Job {} was already terminal", id);
            }
            for id in &result.not_found {
                eprintln!("Job not found: {}", id);
            }

            // Exit with error if any jobs were not found
            if !result.not_found.is_empty() {
                std::process::exit(1);
            }
        }
        JobCommand::Attach { id } => {
            let job = client
                .get_job(&id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("job not found: {}", id))?;
            let session_id = job
                .session_id
                .ok_or_else(|| anyhow::anyhow!("job has no active session"))?;
            super::session::attach(&session_id)?;
        }
        JobCommand::Peek { id } => {
            let job = client
                .get_job(&id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Job not found: {}", id))?;

            // Try main job session first, then fall back to running agent session
            let session_id = job.session_id.clone().or_else(|| {
                job.agents
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
                        "╭────── {} ──────",
                        color::header(&format!("peek: {}", session_id))
                    );
                    print!("{}", output);
                    println!("╰────── {} ──────", color::header("end peek"));
                }
                None => {
                    let short_id = job.id.short(8);
                    let is_terminal =
                        job.step == "done" || job.step == "failed" || job.step == "cancelled";

                    if is_terminal {
                        println!("Job {} is {}. No active session.", short_id, job.step);
                    } else {
                        println!(
                            "No active session for job {} (step: {}, status: {})",
                            short_id, job.step, job.step_status
                        );
                    }
                    println!();
                    println!("Try:");
                    println!("    oj job logs {}", short_id);
                    println!("    oj job show {}", short_id);
                }
            }
        }
        JobCommand::Logs { id, follow, limit } => {
            let (log_path, content) = client.get_job_logs(&id, limit).await?;
            display_log(&log_path, &content, follow, format, "job", &id).await?;
        }
        JobCommand::Prune {
            all,
            failed,
            orphans,
            dry_run,
        } => {
            // Only scope by namespace when explicitly requested via --project.
            // Without this, prune matches `job list` behavior and operates
            // across all namespaces — fixing the bug where auto-resolved namespace
            // silently skipped jobs from other projects.
            let (pruned, skipped) = client
                .job_prune(all, failed, orphans, dry_run, project)
                .await?;

            print_prune_results(
                &pruned,
                skipped,
                dry_run,
                format,
                "job",
                "skipped",
                |entry| {
                    let short_id = entry.id.short(8);
                    format!("{} ({}, {})", entry.name, short_id, entry.step)
                },
            )?;
        }
        JobCommand::Wait { ids, all, timeout } => {
            super::job_wait::handle(ids, all, timeout, client).await?;
        }
    }

    Ok(())
}

/// Print follow-up commands for a job.
pub(crate) fn print_job_commands(short_id: &str) {
    println!("    oj job show {short_id}");
    println!(
        "    oj job wait {short_id}      {}",
        color::muted("# Wait until job ends")
    );
    println!(
        "    oj job logs {short_id} -f   {}",
        color::muted("# Follow logs")
    );
    println!(
        "    oj job peek {short_id}      {}",
        color::muted("# Capture tmux pane")
    );
    println!(
        "    oj job attach {short_id}    {}",
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

/// Returns the sort priority for a variable namespace.
/// Lower values sort first: invoke (0) → workspace (1) → local (2) → var (3) → other (4)
fn var_namespace_priority(name: &str) -> u8 {
    if name.starts_with("invoke.") {
        0
    } else if name.starts_with("workspace.") {
        1
    } else if name.starts_with("local.") {
        2
    } else if name.starts_with("var.") {
        3
    } else {
        4
    }
}

/// Sort variables by namespace priority, then alphabetically within each group.
fn sorted_vars(vars: &HashMap<String, String>) -> Vec<(&String, &String)> {
    let mut sorted: Vec<_> = vars.iter().collect();
    sorted.sort_by(|(a, _), (b, _)| {
        let pri_a = var_namespace_priority(a);
        let pri_b = var_namespace_priority(b);
        pri_a.cmp(&pri_b).then_with(|| a.cmp(b))
    });
    sorted
}

fn is_var_truncated(value: &str, max_len: usize) -> bool {
    let escaped = value.replace('\n', "\\n");
    escaped.chars().count() > max_len
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod tests;
