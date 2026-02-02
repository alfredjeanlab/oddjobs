// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj pipeline` - Pipeline management commands

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::exit_error::ExitError;
use crate::output::{display_log, format_time_ago, should_use_color, OutputFormat};

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
fn parse_key_value(s: &str) -> Result<(String, String), String> {
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

/// Map a pipeline's step/status to a sort group: 0 = active, 1 = failed, 2 = terminal.
pub(crate) fn status_group(step: &str, step_status: &str) -> u8 {
    match step {
        "failed" => 1,
        "done" => 2,
        _ => match step_status {
            "Running" | "Pending" | "Waiting" => 0,
            "Failed" => 1,
            "Completed" => 2,
            _ => 2,
        },
    }
}

enum PipelineOutcome {
    Done,
    Failed(String),
    Cancelled,
}

pub async fn handle(
    command: PipelineCommand,
    client: &DaemonClient,
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

            // Sort: active first, then failed, then completed — most recent first within each group
            pipelines.sort_by(|a, b| {
                let ga = status_group(&a.step, &a.step_status);
                let gb = status_group(&b.step, &b.step_status);
                ga.cmp(&gb).then(b.created_at_ms.cmp(&a.created_at_ms))
            });

            // Limit
            let total = pipelines.len();
            let effective_limit = if no_limit { total } else { limit };
            let truncated = total > effective_limit;
            if truncated {
                pipelines.truncate(effective_limit);
            }

            match format {
                OutputFormat::Text => {
                    if pipelines.is_empty() {
                        println!("No pipelines");
                    } else {
                        // Show PROJECT column only when multiple namespaces present
                        let namespaces: std::collections::HashSet<&str> =
                            pipelines.iter().map(|p| p.namespace.as_str()).collect();
                        let show_project =
                            namespaces.len() > 1 || namespaces.iter().any(|n| !n.is_empty());

                        if show_project {
                            println!(
                                "{:<12} {:<16} {:<16} {:<10} {:<15} {:<10} STATUS",
                                "ID", "PROJECT", "NAME", "KIND", "STEP", "UPDATED"
                            );
                        } else {
                            println!(
                                "{:<12} {:<20} {:<10} {:<15} {:<10} STATUS",
                                "ID", "NAME", "KIND", "STEP", "UPDATED"
                            );
                        }
                        for p in &pipelines {
                            let updated_ago = format_time_ago(p.updated_at_ms);
                            if show_project {
                                println!(
                                    "{:<12} {:<16} {:<16} {:<10} {:<15} {:<10} {}",
                                    &p.id[..12.min(p.id.len())],
                                    &p.namespace[..16.min(p.namespace.len())],
                                    &p.name[..16.min(p.name.len())],
                                    &p.kind[..10.min(p.kind.len())],
                                    p.step,
                                    updated_ago,
                                    p.step_status
                                );
                            } else {
                                println!(
                                    "{:<12} {:<20} {:<10} {:<15} {:<10} {}",
                                    &p.id[..12.min(p.id.len())],
                                    &p.name[..20.min(p.name.len())],
                                    &p.kind[..10.min(p.kind.len())],
                                    p.step,
                                    updated_ago,
                                    p.step_status
                                );
                            }
                        }
                    }

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
        PipelineCommand::Show { id } => {
            let pipeline = client.get_pipeline(&id).await?;

            match format {
                OutputFormat::Text => {
                    if let Some(p) = pipeline {
                        println!("Pipeline: {}", p.id);
                        println!("  Name: {}", p.name);
                        if !p.namespace.is_empty() {
                            println!("  Project: {}", p.namespace);
                        }
                        println!("  Kind: {}", p.kind);
                        println!("  Status: {}", p.step_status);

                        if !p.steps.is_empty() {
                            println!();
                            println!("  Steps:");
                            for step in &p.steps {
                                let duration =
                                    format_duration(step.started_at_ms, step.finished_at_ms);
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
                                println!("    {:<12} {:<8} {}", step.name, duration, status);
                            }
                        }

                        if !p.agents.is_empty() {
                            println!();
                            println!("  Agents:");
                            for agent in &p.agents {
                                let summary = format_agent_summary(agent);
                                let session_id = truncate(&agent.agent_id, 24);
                                if summary.is_empty() {
                                    println!(
                                        "    {:<12} {:<12} {}",
                                        agent.step_name, agent.status, session_id,
                                    );
                                } else {
                                    println!(
                                        "    {:<12} {:<12} {} ({})",
                                        agent.step_name, agent.status, summary, session_id,
                                    );
                                }
                            }
                        }

                        println!();
                        if let Some(session) = &p.session_id {
                            println!("  Session: {}", session);
                        }
                        if let Some(ws) = &p.workspace_path {
                            println!("  Workspace: {}", ws.display());
                        }
                        if let Some(error) = &p.error {
                            println!("  Error: {}", error);
                        }
                        if !p.vars.is_empty() {
                            println!("  Vars:");
                            for (k, v) in &p.vars {
                                println!("    {}: {}", k, v);
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

            match session_id {
                Some(session_id) => {
                    let with_color = should_use_color();
                    let output = client.peek_session(&session_id, with_color).await?;
                    println!("╭──── peek: {} ────", session_id);
                    print!("{}", output);
                    println!("╰──── end peek ────");
                }
                None => {
                    let short_id = &pipeline.id[..12.min(pipeline.id.len())];
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
        PipelineCommand::Wait { ids, all, timeout } => {
            let timeout_dur = timeout.map(|s| parse_duration(&s)).transpose()?;
            let poll_ms = std::env::var("OJ_WAIT_POLL_MS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1000);
            let poll_interval = Duration::from_millis(poll_ms);
            let start = Instant::now();

            let mut finished: HashMap<String, PipelineOutcome> = HashMap::new();
            let mut canonical_ids: HashMap<String, String> = HashMap::new();
            let mut step_trackers: HashMap<String, StepTracker> = HashMap::new();
            let show_prefix = ids.len() > 1;

            loop {
                for input_id in &ids {
                    if finished.contains_key(input_id) {
                        continue;
                    }
                    let detail = client.get_pipeline(input_id).await?;
                    match detail {
                        None => {
                            return Err(ExitError::new(
                                3,
                                format!("Pipeline not found: {}", input_id),
                            )
                            .into());
                        }
                        Some(p) => {
                            canonical_ids
                                .entry(input_id.clone())
                                .or_insert_with(|| p.id.clone());

                            let tracker =
                                step_trackers
                                    .entry(input_id.clone())
                                    .or_insert(StepTracker {
                                        printed_count: 0,
                                        printed_started: false,
                                    });
                            let mut stdout = std::io::stdout();
                            print_step_progress(&p, tracker, show_prefix, &mut stdout);

                            let outcome = match p.step.as_str() {
                                "done" => Some(PipelineOutcome::Done),
                                "failed" => Some(PipelineOutcome::Failed(
                                    p.error.clone().unwrap_or_else(|| "unknown error".into()),
                                )),
                                "cancelled" => Some(PipelineOutcome::Cancelled),
                                _ => None,
                            };
                            if let Some(outcome) = outcome {
                                let short_id = &canonical_ids[input_id][..8];
                                match &outcome {
                                    PipelineOutcome::Done => {
                                        println!("Pipeline {} ({}) completed", p.name, short_id);
                                    }
                                    PipelineOutcome::Failed(msg) => {
                                        eprintln!(
                                            "Pipeline {} ({}) failed: {}",
                                            p.name, short_id, msg
                                        );
                                    }
                                    PipelineOutcome::Cancelled => {
                                        eprintln!(
                                            "Pipeline {} ({}) was cancelled",
                                            p.name, short_id
                                        );
                                    }
                                }
                                finished.insert(input_id.clone(), outcome);
                            }
                        }
                    }
                }

                if all {
                    if finished.len() == ids.len() {
                        break;
                    }
                } else if !finished.is_empty() {
                    break;
                }

                if let Some(t) = timeout_dur {
                    if start.elapsed() >= t {
                        return Err(ExitError::new(
                            2,
                            "Timeout waiting for pipeline(s)".to_string(),
                        )
                        .into());
                    }
                }

                tokio::time::sleep(poll_interval).await;
            }

            let any_failed = finished
                .values()
                .any(|o| matches!(o, PipelineOutcome::Failed(_)));
            let any_cancelled = finished
                .values()
                .any(|o| matches!(o, PipelineOutcome::Cancelled));
            if any_failed {
                return Err(ExitError::new(1, String::new()).into());
            }
            if any_cancelled {
                return Err(ExitError::new(4, String::new()).into());
            }
        }
    }

    Ok(())
}

/// Tracks step progress for a single pipeline during wait polling.
struct StepTracker {
    /// Number of steps we've already printed final transitions for.
    printed_count: usize,
    /// Whether we've printed a "started" line for the current (not-yet-final) step.
    printed_started: bool,
}

/// Print step transitions that occurred since the last poll.
fn print_step_progress(
    detail: &oj_daemon::PipelineDetail,
    tracker: &mut StepTracker,
    show_pipeline_prefix: bool,
    out: &mut impl std::io::Write,
) {
    let prefix = if show_pipeline_prefix {
        format!("[{}] ", detail.name)
    } else {
        String::new()
    };

    for (i, step) in detail.steps.iter().enumerate() {
        if i < tracker.printed_count {
            continue;
        }

        let is_terminal = matches!(step.outcome.as_str(), "completed" | "failed");

        if is_terminal {
            // Print "started" for steps we haven't announced yet (skipped running state)
            if i == tracker.printed_count && !tracker.printed_started {
                // Step completed between polls without us seeing "running" — don't print started
                // for instant steps, just print the final outcome directly.
            }

            let elapsed = format_duration(step.started_at_ms, step.finished_at_ms);
            match step.outcome.as_str() {
                "completed" => {
                    let _ = writeln!(out, "{}{} completed ({})", prefix, step.name, elapsed);
                }
                "failed" => {
                    let suffix = match &step.detail {
                        Some(d) if !d.is_empty() => format!(" - {}", d),
                        _ => String::new(),
                    };
                    let _ = writeln!(
                        out,
                        "{}{} failed ({}){}",
                        prefix, step.name, elapsed, suffix
                    );
                }
                _ => unreachable!(),
            }
            tracker.printed_count = i + 1;
            tracker.printed_started = false;
        } else if step.outcome == "running" && !tracker.printed_started {
            let _ = writeln!(out, "{}{} started", prefix, step.name);
            tracker.printed_started = true;
        } else if step.outcome == "waiting" && !tracker.printed_started {
            let reason = step.detail.as_deref().unwrap_or("waiting");
            let _ = writeln!(out, "{}{} waiting ({})", prefix, step.name, reason);
            tracker.printed_started = true;
        }
    }
}

/// Print follow-up commands for a pipeline.
pub(crate) fn print_pipeline_commands(short_id: &str) {
    println!("    oj pipeline show {short_id}");
    println!("    oj pipeline wait {short_id}      # Wait until pipeline ends");
    println!("    oj pipeline logs {short_id} -f   # Follow logs");
    println!("    oj pipeline peek {short_id}      # Capture tmux pane");
    println!("    oj pipeline attach {short_id}    # Attach to tmux");
}

fn format_duration(started_ms: u64, finished_ms: Option<u64>) -> String {
    let end = finished_ms.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    });
    let elapsed_secs = (end.saturating_sub(started_ms)) / 1000;
    if elapsed_secs < 60 {
        format!("{}s", elapsed_secs)
    } else if elapsed_secs < 3600 {
        format!("{}m {}s", elapsed_secs / 60, elapsed_secs % 60)
    } else {
        format!("{}h {}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
    }
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

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
