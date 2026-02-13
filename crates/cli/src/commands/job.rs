// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj job` - Job management commands

use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, Subcommand};

use oj_core::StepOutcomeKind;

use crate::client::{ClientKind, DaemonClient};
use crate::color;
use crate::output::{
    apply_limit, display_log, filter_by_project, format_or_json, format_time_ago,
    handle_list_with_limit, poll_log_follow, print_batch_action_results, print_capture_frame,
    print_prune_results, OutputFormat,
};
use crate::table::{Column, Table};

pub(crate) use super::job_display::print_job_commands;
use super::job_display::{
    format_agent_summary, format_var_value, group_vars_by_scope, is_var_truncated, truncate,
};

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
        /// Job ID or name. Required unless --all is used.
        id: Option<String>,

        /// Message for nudge/recovery (required for agent steps)
        #[arg(short = 'm', long)]
        message: Option<String>,

        /// Job variables to set (can be repeated: --var key=value)
        #[arg(long = "var", value_parser = parse_key_value)]
        var: Vec<(String, String)>,

        /// Kill running agent and restart (still preserves conversation via --resume)
        #[arg(long)]
        kill: bool,

        /// Resume all resumable jobs (waiting/failed/pending)
        #[arg(long)]
        all: bool,
    },
    /// Cancel one or more running jobs
    Cancel {
        /// Job IDs or names (prefix match)
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Suspend one or more running jobs (preserves workspace for later resume)
    Suspend {
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
    /// Peek at the active agent session for a job
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

impl JobCommand {
    pub fn client_kind(&self) -> ClientKind {
        match self {
            Self::List { .. }
            | Self::Show { .. }
            | Self::Logs { .. }
            | Self::Peek { .. }
            | Self::Wait { .. }
            | Self::Attach { .. } => ClientKind::Query,
            _ => ClientKind::Action,
        }
    }
}

/// Parse a key=value string for input arguments.
pub(crate) fn parse_key_value(s: &str) -> Result<(String, String), String> {
    let pos =
        s.find('=').ok_or_else(|| format!("invalid input format '{}': must be key=value", s))?;
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
            let n: u64 =
                current_num.parse().map_err(|_| anyhow::anyhow!("invalid duration: {}", s))?;
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
        let n: u64 = current_num.parse().map_err(|_| anyhow::anyhow!("invalid duration: {}", s))?;
        total_secs += n;
    }
    if total_secs == 0 {
        anyhow::bail!("duration must be > 0: {}", s);
    }
    Ok(Duration::from_secs(total_secs))
}

pub(crate) fn format_job_list(out: &mut (impl Write + ?Sized), jobs: &[oj_wire::JobSummary]) {
    if jobs.is_empty() {
        let _ = writeln!(out, "No jobs");
        return;
    }

    // Show RETRIES column only when any job has retries
    let show_retries = jobs.iter().any(|p| p.retries > 0);

    // Build columns — RETRIES is conditionally inserted
    let mut cols = vec![
        Column::muted("ID"),
        Column::left("PROJECT"),
        Column::left("NAME"),
        Column::left("KIND"),
        Column::left("STEP"),
        Column::left("UPDATED"),
    ];
    // Insert RETRIES before STATUS (which will be last)
    if show_retries {
        cols.push(Column::left("RETRIES"));
    }
    cols.push(Column::status("STATUS"));

    let mut table = Table::new(cols);

    for p in jobs {
        let id = p.id.short(8).to_string();
        let ns = if p.project.is_empty() { "-" } else { &p.project };
        let updated = format_time_ago(p.updated_at_ms);

        let mut cells =
            vec![id, ns.to_string(), p.name.clone(), p.kind.clone(), p.step.clone(), updated];
        if show_retries {
            cells.push(p.retries.to_string());
        }
        cells.push(p.step_status.to_string());
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
        JobCommand::List { name, status, limit, no_limit } => {
            let mut jobs = client.list_jobs().await?;
            filter_by_project(&mut jobs, project, |p| &p.project);

            // Filter by name substring
            if let Some(ref pat) = name {
                let pat_lower = pat.to_lowercase();
                jobs.retain(|p| p.name.to_lowercase().contains(&pat_lower));
            }

            // Filter by status
            if let Some(ref st) = status {
                let st_lower = st.to_lowercase();
                jobs.retain(|p| {
                    p.step_status.to_string() == st_lower || p.step.to_lowercase() == st_lower
                });
            }

            jobs.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
            let truncation = apply_limit(&mut jobs, limit, no_limit);

            handle_list_with_limit(format, &jobs, "No jobs", truncation, |items, out| {
                format_job_list(out, items);
            })?;
        }
        JobCommand::Show { id, verbose } => {
            let job = client.get_job(&id).await?;
            format_or_json(format, &job, || {
                if let Some(p) = &job {
                    println!("{} {}", color::header("Job:"), p.id);
                    println!("  {} {}", color::context("Name:"), p.name);
                    if !p.project.is_empty() {
                        println!("  {} {}", color::context("Project:"), p.project);
                    }
                    println!("  {} {}", color::context("Kind:"), p.kind);
                    println!(
                        "  {} {}",
                        color::context("Status:"),
                        color::status(&p.step_status.to_string())
                    );
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
                            let status = match (&step.outcome, &step.detail) {
                                (StepOutcomeKind::Failed | StepOutcomeKind::Waiting, Some(d)) => {
                                    format!("{} ({})", step.outcome, truncate(d, 40))
                                }
                                _ => step.outcome.to_string(),
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
                        println!("  {}", color::header("Crew:"));
                        for agent in &p.agents {
                            let summary = format_agent_summary(agent);
                            let short_id = agent.agent_id.short(8);
                            if summary.is_empty() {
                                println!(
                                    "    {:<12} {} {}",
                                    agent.step_name,
                                    color::status(&format!("{:<12}", &agent.status)),
                                    color::muted(short_id),
                                );
                            } else {
                                println!(
                                    "    {:<12} {} {} ({})",
                                    agent.step_name,
                                    color::status(&format!("{:<12}", &agent.status)),
                                    summary,
                                    color::muted(short_id),
                                );
                            }
                        }
                    }

                    if !p.vars.is_empty() {
                        println!();
                        println!("  {}", color::header("Variables:"));
                        let sorted_vars = group_vars_by_scope(&p.vars);
                        if verbose {
                            for (k, v) in &sorted_vars {
                                if v.contains('\n') {
                                    println!("    {}", color::context(&format!("{}:", k)));
                                    for line in v.lines() {
                                        println!("      {}", line);
                                    }
                                } else {
                                    println!("    {} {}", color::context(&format!("{}:", k)), v);
                                }
                            }
                        } else {
                            for (k, v) in &sorted_vars {
                                println!(
                                    "    {} {}",
                                    color::context(&format!("{}:", k)),
                                    format_var_value(v, 80)
                                );
                            }
                            let any_truncated = p.vars.values().any(|v| is_var_truncated(v, 80));
                            if any_truncated {
                                println!();
                                println!(
                                    "  {}",
                                    color::context("hint: use --verbose to show full variables")
                                );
                            }
                        }
                    }
                } else {
                    println!("Job not found: {}", id);
                }
            })?;
        }
        JobCommand::Resume { id, message, var, kill, all } => {
            if all {
                if id.is_some() || message.is_some() || !var.is_empty() {
                    anyhow::bail!("--all cannot be combined with a job ID, --message, or --var");
                }
                let (resumed, skipped) = client.job_resume_all(kill).await?;

                let obj = serde_json::json!({
                    "resumed": resumed,
                    "skipped": skipped,
                });
                format_or_json(format, &obj, || {
                    if resumed.is_empty() && skipped.is_empty() {
                        println!("No resumable jobs found");
                    } else {
                        for id in &resumed {
                            println!("Resumed job {}", id);
                        }
                        for (id, reason) in &skipped {
                            println!("Skipped job {} ({})", id, reason);
                        }
                    }
                })?;
            } else {
                let id =
                    id.ok_or_else(|| anyhow::anyhow!("Either provide a job ID or use --all"))?;
                let var_map: HashMap<String, String> = var.into_iter().collect();
                client.job_resume(&id, message.as_deref(), &var_map, kill).await?;
                if !var_map.is_empty() {
                    println!("Updated vars and resumed job {}", id);
                } else {
                    println!("Resumed job {}", id);
                }
            }
        }
        JobCommand::Cancel { ids } => {
            let r = client.job_cancel(&ids).await?;
            print_batch_action_results(
                &r.cancelled,
                "Cancelled",
                &r.already_terminal,
                &r.not_found,
            );
        }
        JobCommand::Suspend { ids } => {
            let r = client.job_suspend(&ids).await?;
            print_batch_action_results(
                &r.suspended,
                "Suspended",
                &r.already_terminal,
                &r.not_found,
            );
        }
        JobCommand::Attach { id } => {
            let job = client
                .get_job(&id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("job not found: {}", id))?;
            // Find running agent to attach to
            let agent_id = job
                .agents
                .iter()
                .find(|a| a.status == "running")
                .map(|a| a.agent_id.clone())
                .ok_or_else(|| anyhow::anyhow!("job has no active agent session"))?;
            crate::daemon_process::coop_attach(agent_id.as_str())?;
        }
        JobCommand::Peek { id } => {
            let job = client
                .get_job(&id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Job not found: {}", id))?;

            let short_id = job.id.short(8);

            // Try saved capture: find agent_id from current step, then
            // fall back to the most recent agent in the job.
            let agent_id = job
                .agents
                .iter()
                .find(|a| a.status == "running")
                .map(|a| &a.agent_id)
                .or_else(|| {
                    job.steps.iter().rfind(|s| s.name == job.step).and_then(|s| s.agent_id.as_ref())
                })
                .or_else(|| job.agents.last().map(|a| &a.agent_id));
            if let Some(aid) = agent_id {
                if let Some(content) = super::agent::display::try_read_agent_capture(aid) {
                    let label = aid.short(8);
                    print_capture_frame(label, &content);
                    return Ok(());
                }
            }

            let is_terminal = job.step == "done"
                || job.step == "failed"
                || job.step == "cancelled"
                || job.step == "suspended";

            if job.step == "suspended" {
                println!(
                    "Job {} is suspended. Use `oj resume {}` to continue.",
                    short_id, short_id
                );
            } else if is_terminal {
                println!("Job {} is {}. No active agent.", short_id, job.step);
            } else {
                println!(
                    "No active agent for job {} (step: {}, status: {})",
                    short_id, job.step, job.step_status
                );
            }
            println!();
            println!("Try:");
            println!("    oj job logs {}", short_id);
            println!("    oj job show {}", short_id);
        }
        JobCommand::Logs { id, follow, limit } => {
            let (log_path, content, offset) = client.get_job_logs(&id, limit, 0).await?;
            if let Some(off) =
                display_log(&log_path, &content, follow, offset, format, "job", &id).await?
            {
                let id = id.clone();
                poll_log_follow(off, |o| {
                    let id = id.clone();
                    async move {
                        let (_, c, new_off) = client.get_job_logs(&id, 0, o).await?;
                        Ok((c, new_off))
                    }
                })
                .await?;
            }
        }
        JobCommand::Prune { all, failed, orphans, dry_run } => {
            // Only scope by project when explicitly requested via --project.
            // Without this, prune matches `job list` behavior and operates
            // across all namespaces — fixing the bug where auto-resolved project
            // silently skipped jobs from other projects.
            let (pruned, skipped) =
                client.job_prune(all, failed, orphans, dry_run, project).await?;

            print_prune_results(&pruned, skipped, dry_run, format, "job", "skipped", |entry| {
                let short_id = entry.id.short(8);
                format!("{} ({}, {})", entry.name, short_id, entry.step)
            })?;
        }
        JobCommand::Wait { ids, all, timeout } => {
            super::job_wait::handle(ids, all, timeout, client).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod tests;
