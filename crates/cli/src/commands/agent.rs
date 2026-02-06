// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent management commands

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

use oj_core::{AgentId, Event, PromptType, QuestionData, ShortId};

use crate::client::{ClientKind, DaemonClient};
use crate::color;
use crate::exit_error::ExitError;
use crate::output::{
    display_log, print_peek_frame, print_prune_results, should_use_color, OutputFormat,
};
use crate::table::{project_cell, should_show_project, Column, Table};

use super::job::parse_duration;

#[derive(Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommand,
}

#[derive(Subcommand)]
pub enum AgentCommand {
    /// List agents across all jobs
    List {
        /// Filter by job ID (or prefix)
        #[arg(long)]
        job: Option<String>,

        /// Filter by status (e.g. "running", "completed", "failed", "waiting")
        #[arg(long)]
        status: Option<String>,

        /// Maximum number of agents to show (default: 20)
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,

        /// Show all agents (no limit)
        #[arg(long, conflicts_with = "limit")]
        no_limit: bool,
    },
    /// Show detailed info for a single agent
    Show {
        /// Agent ID (or prefix)
        id: String,
    },
    /// Send a message to a running agent
    Send {
        /// Agent ID or job ID (or prefix)
        agent_id: String,
        /// Message to send
        message: String,
    },
    /// View agent activity log
    Logs {
        /// Job ID (or prefix)
        id: String,
        /// Show only a specific step's log
        #[arg(long, short = 's')]
        step: Option<String>,
        /// Stream live activity (like tail -f)
        #[arg(long, short)]
        follow: bool,
        /// Number of recent lines to show per step (default: 50)
        #[arg(short = 'n', long, default_value = "50")]
        limit: usize,
    },
    /// Block until a specific agent reaches a terminal or idle state
    Wait {
        /// Agent ID (or prefix)
        agent_id: String,
        /// Timeout duration (e.g. "5m", "30s", "1h")
        #[arg(long)]
        timeout: Option<String>,
    },
    /// Peek at an agent's tmux session output
    Peek {
        /// Agent ID (or prefix)
        id: String,
    },
    /// Attach to an agent's tmux session
    Attach {
        /// Agent ID (or prefix)
        id: String,
    },
    /// Remove agent logs from completed/failed/cancelled jobs
    Prune {
        /// Remove all agent logs from terminal jobs regardless of age
        #[arg(long)]
        all: bool,
        /// Show what would be pruned without doing it
        #[arg(long)]
        dry_run: bool,
    },
    /// Resume a dead agent's session (re-spawn with --resume to preserve conversation)
    Resume {
        /// Agent ID (or prefix). Required unless --all is used.
        id: Option<String>,
        /// Force kill the current tmux session before resuming
        #[arg(long)]
        kill: bool,
        /// Resume all agents that have dead sessions
        #[arg(long)]
        all: bool,
    },
    /// Hook subcommands for Claude Code integration
    Hook {
        #[command(subcommand)]
        hook: HookCommand,
    },
}

impl AgentCommand {
    pub fn client_kind(&self) -> ClientKind {
        match self {
            Self::Send { .. } | Self::Resume { .. } | Self::Prune { .. } => ClientKind::Action,
            Self::Hook { .. } => ClientKind::Signal,
            _ => ClientKind::Query,
        }
    }
}

#[derive(Subcommand)]
pub enum HookCommand {
    /// Stop hook handler - gates agent completion
    Stop {
        /// Agent ID to check
        agent_id: String,
    },
    /// PreToolUse hook handler - detects plan/question tools and transitions to Prompting
    Pretooluse {
        /// Agent ID to emit prompt event for
        agent_id: String,
    },
    /// Notification hook handler - detects idle_prompt and permission_prompt
    Notify {
        /// Agent ID to emit state events for
        #[arg(long)]
        agent_id: String,
    },
}

/// Input from Claude Code PreToolUse hook (subset of fields we care about)
#[derive(Deserialize)]
struct PreToolUseInput {
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: Option<serde_json::Value>,
}

/// Input from Claude Code Stop hook (subset of fields we care about)
#[derive(Deserialize)]
struct StopHookInput {
    #[serde(default)]
    stop_hook_active: bool,
    // Other fields available: session_id, transcript_path, cwd, permission_mode, hook_event_name
}

/// Output to Claude Code Stop hook
#[derive(Serialize)]
struct StopHookOutput {
    decision: String,
    reason: String,
}

/// Input from Claude Code Notification hook (subset of fields we care about)
#[derive(Debug, Deserialize)]
struct NotificationHookInput {
    #[serde(default)]
    notification_type: String,
}

pub async fn handle(
    command: AgentCommand,
    client: &DaemonClient,
    _namespace: &str,
    project_filter: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        AgentCommand::List {
            job,
            status,
            limit,
            no_limit,
        } => {
            let mut agents = client
                .list_agents(job.as_deref(), status.as_deref())
                .await?;

            // Filter by explicit --project flag (OJ_NAMESPACE is NOT used for filtering)
            if let Some(proj) = project_filter {
                agents.retain(|a| a.namespace.as_deref() == Some(proj));
            }

            let total = agents.len();
            let display_limit = if no_limit { total } else { limit };
            let agents: Vec<_> = agents.into_iter().take(display_limit).collect();
            let remaining = total.saturating_sub(display_limit);

            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&agents)?);
                }
                OutputFormat::Text => {
                    if agents.is_empty() {
                        println!("No agents found");
                    } else {
                        let show_project = should_show_project(
                            agents.iter().map(|a| a.namespace.as_deref().unwrap_or("")),
                        );

                        let mut cols = vec![Column::muted("ID").with_max(8), Column::left("KIND")];
                        if show_project {
                            cols.push(Column::left("PROJECT"));
                        }
                        cols.extend([
                            Column::left("JOB").with_max(8),
                            Column::left("STEP"),
                            Column::status("STATUS"),
                            Column::right("READ"),
                            Column::right("WRITE"),
                            Column::right("CMDS"),
                        ]);
                        let mut table = Table::new(cols);

                        for a in &agents {
                            let name = a.agent_name.as_deref().unwrap_or("-").to_string();
                            let job_col = if a.job_id.is_empty() {
                                "-".to_string()
                            } else {
                                a.job_id.clone()
                            };
                            let step_col = if a.step_name.is_empty() {
                                "-".to_string()
                            } else {
                                a.step_name.clone()
                            };
                            let mut cells = vec![a.agent_id.clone(), name];
                            if show_project {
                                cells.push(project_cell(a.namespace.as_deref().unwrap_or("")));
                            }
                            cells.extend([
                                job_col,
                                step_col,
                                a.status.clone(),
                                a.files_read.to_string(),
                                a.files_written.to_string(),
                                a.commands_run.to_string(),
                            ]);
                            table.row(cells);
                        }
                        table.render(&mut std::io::stdout());
                    }
                    if remaining > 0 {
                        println!(
                            "\n... {} more not shown. Use --no-limit or -n N to see more.",
                            remaining
                        );
                    }
                }
            }
        }
        AgentCommand::Show { id } => {
            let agent = client.get_agent(&id).await?;

            match format {
                OutputFormat::Text => {
                    if let Some(a) = agent {
                        println!("{} {}", color::header("Agent:"), a.agent_id);
                        println!(
                            "  {} {}",
                            color::context("Name:"),
                            a.agent_name.as_deref().unwrap_or("-")
                        );
                        if let Some(ref ns) = a.namespace {
                            if !ns.is_empty() {
                                println!("  {} {}", color::context("Project:"), ns);
                            }
                        }
                        if a.job_id.is_empty() {
                            println!("  {} standalone", color::context("Source:"));
                        } else {
                            println!("  {} {} ({})", color::context("Job:"), a.job_id, a.job_name);
                            println!("  {} {}", color::context("Step:"), a.step_name);
                        }
                        println!(
                            "  {} {}",
                            color::context("Status:"),
                            color::status(&a.status)
                        );

                        println!();
                        println!("  {}", color::header("Activity:"));
                        println!("    Files read: {}", a.files_read);
                        println!("    Files written: {}", a.files_written);
                        println!("    Commands run: {}", a.commands_run);

                        println!();
                        if let Some(ref session) = a.session_id {
                            println!("  {} {}", color::context("Session:"), session);
                        }
                        if let Some(ref ws) = a.workspace_path {
                            println!("  {} {}", color::context("Workspace:"), ws.display());
                        }
                        println!(
                            "  {} {}",
                            color::context("Started:"),
                            crate::output::format_time_ago(a.started_at_ms)
                        );
                        println!(
                            "  {} {}",
                            color::context("Updated:"),
                            crate::output::format_time_ago(a.updated_at_ms)
                        );
                        if let Some(ref err) = a.error {
                            println!();
                            println!("  {} {}", color::context("Error:"), err);
                        } else if let Some(ref reason) = a.exit_reason {
                            if reason.starts_with("failed") || reason == "gone" {
                                println!();
                                println!("  {} {}", color::context("Error:"), reason);
                            }
                        }
                    } else {
                        println!("Agent not found: {}", id);
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&agent)?);
                }
            }
        }
        AgentCommand::Peek { id } => {
            let agent = client
                .get_agent(&id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Agent not found: {}", id))?;

            let session_id = agent
                .session_id
                .ok_or_else(|| anyhow::anyhow!("Agent has no active session"))?;

            let with_color = should_use_color();
            match client.peek_session(&session_id, with_color).await {
                Ok(output) => {
                    print_peek_frame(&session_id, &output);
                }
                Err(crate::client::ClientError::Rejected(msg))
                    if msg.starts_with("Session not found") =>
                {
                    let short_id = agent.agent_id.short(8);
                    let is_terminal = agent.status == "completed"
                        || agent.status == "failed"
                        || agent.status == "cancelled";

                    if is_terminal {
                        println!("Agent {} is {}. No active session.", short_id, agent.status);
                    } else {
                        println!(
                            "No active session for agent {} (status: {})",
                            short_id, agent.status
                        );
                    }
                    println!();
                    println!("Try:");
                    println!("    oj agent logs {}", short_id);
                    println!("    oj agent show {}", short_id);
                }
                Err(e) => return Err(e.into()),
            }
        }
        AgentCommand::Attach { id } => {
            let agent = client
                .get_agent(&id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Agent not found: {}", id))?;

            let session_id = agent
                .session_id
                .ok_or_else(|| anyhow::anyhow!("Agent has no active session"))?;

            super::session::attach(&session_id)?;
        }
        AgentCommand::Send { agent_id, message } => {
            client.agent_send(&agent_id, &message).await?;
            println!("Sent to agent {}", agent_id);
        }
        AgentCommand::Logs {
            id,
            step,
            follow,
            limit,
        } => {
            let (log_path, content, _steps) =
                client.get_agent_logs(&id, step.as_deref(), limit).await?;
            display_log(&log_path, &content, follow, format, "agent", &id).await?;
        }
        AgentCommand::Wait { agent_id, timeout } => {
            handle_wait(&agent_id, timeout.as_deref(), client).await?;
        }
        AgentCommand::Prune { all, dry_run } => {
            let (pruned, skipped) = client.agent_prune(all, dry_run).await?;

            print_prune_results(
                &pruned,
                skipped,
                dry_run,
                format,
                "agent",
                "job(s) skipped",
                |entry| {
                    if entry.job_id.is_empty() {
                        // Standalone agent run
                        format!("agent {} ({})", entry.agent_id.short(8), entry.step_name)
                    } else {
                        // Job-embedded agent
                        let short_pid = entry.job_id.short(8);
                        format!(
                            "agent {} ({}, {})",
                            entry.agent_id.short(8),
                            short_pid,
                            entry.step_name
                        )
                    }
                },
            )?;
        }
        AgentCommand::Resume { id, kill, all } => {
            if !all && id.is_none() {
                return Err(anyhow::anyhow!("Either provide an agent ID or use --all"));
            }
            let agent_id = id.unwrap_or_default();
            let (resumed, skipped) = client.agent_resume(&agent_id, kill, all).await?;

            match format {
                OutputFormat::Text => {
                    for aid in &resumed {
                        println!("Resumed agent {}", aid.short(8));
                    }
                    for (aid, reason) in &skipped {
                        println!("Skipped agent {}: {}", aid.short(8), reason);
                    }
                    if resumed.is_empty() && skipped.is_empty() {
                        println!("No agents to resume");
                    }
                }
                OutputFormat::Json => {
                    let obj = serde_json::json!({
                        "resumed": resumed,
                        "skipped": skipped,
                    });
                    println!("{}", serde_json::to_string_pretty(&obj)?);
                }
            }
        }
        AgentCommand::Hook { hook } => match hook {
            HookCommand::Stop { agent_id } => {
                handle_stop_hook(&agent_id, client).await?;
            }
            HookCommand::Pretooluse { agent_id } => {
                handle_pretooluse_hook(&agent_id, client).await?;
            }
            HookCommand::Notify { agent_id } => {
                handle_notify_hook(&agent_id, client).await?;
            }
        },
    }

    Ok(())
}

/// Resolve the OJ state directory from environment or default.
fn get_state_dir() -> PathBuf {
    crate::env::state_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Format current UTC time as an ISO 8601 timestamp.
fn utc_timestamp() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Convert epoch seconds to date-time components
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since epoch to Y-M-D (civil calendar from days)
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Append a timestamped line to the agent log file.
/// Failures are silently ignored since logging should not block the hook.
fn append_agent_log(agent_id: &str, message: &str) {
    let log_path = get_state_dir()
        .join("logs/agent")
        .join(format!("{agent_id}.log"));
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let timestamp = utc_timestamp();
    let line = format!("{timestamp} stop-hook: {message}\n");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Find an agent by ID (or prefix) across all jobs.
/// Returns `(job_id, agent_id)` on match, or None.
async fn find_agent(
    client: &DaemonClient,
    agent_id: &str,
) -> Result<Option<(String, String)>, anyhow::Error> {
    let jobs = client.list_jobs().await?;
    for summary in &jobs {
        if let Some(detail) = client.get_job(&summary.id).await? {
            for agent in &detail.agents {
                if agent.agent_id == agent_id || agent.agent_id.starts_with(agent_id) {
                    return Ok(Some((summary.id.clone(), agent.agent_id.clone())));
                }
            }
        }
    }
    Ok(None)
}

async fn handle_wait(agent_id: &str, timeout: Option<&str>, client: &DaemonClient) -> Result<()> {
    let timeout_dur = timeout.map(parse_duration).transpose()?;
    let mut poller = crate::poll::Poller::new(crate::client::wait_poll_interval(), timeout_dur);
    let start = Instant::now();

    // Resolve agent to a job on first iteration; re-scan if not found yet
    let mut resolved_job_id: Option<String> = None;
    let mut resolved_agent_id: Option<String> = None;

    loop {
        // If we haven't found the agent yet, search for it
        if resolved_job_id.is_none() {
            if let Some((pid, aid)) = find_agent(client, agent_id).await? {
                resolved_job_id = Some(pid);
                resolved_agent_id = Some(aid);
            }
        }

        let job_id = match &resolved_job_id {
            Some(id) => id.clone(),
            None => {
                // On first poll with no match, give a grace period for the agent to appear
                if start.elapsed() > Duration::from_secs(10) {
                    return Err(ExitError::new(3, format!("Agent not found: {}", agent_id)).into());
                }
                match poller.tick().await {
                    crate::poll::Tick::Ready => continue,
                    crate::poll::Tick::Timeout => {
                        return Err(ExitError::new(
                            2,
                            format!("Timeout waiting for agent {}", agent_id),
                        )
                        .into());
                    }
                    crate::poll::Tick::Interrupted => {
                        return Err(ExitError::new(130, String::new()).into());
                    }
                }
            }
        };

        let full_agent_id = resolved_agent_id.as_deref().unwrap_or(agent_id);

        let detail = client.get_job(&job_id).await?;
        match detail {
            None => {
                return Err(ExitError::new(3, format!("Job {} disappeared", job_id)).into());
            }
            Some(p) => {
                // Find our specific agent in the job
                let agent = p.agents.iter().find(|a| a.agent_id == full_agent_id);

                match agent {
                    Some(agent) => {
                        // Check agent-level terminal/idle states
                        match agent.status.as_str() {
                            "completed" => {
                                println!("Agent {} completed", full_agent_id);
                                break;
                            }
                            "waiting" => {
                                println!("Agent {} waiting", full_agent_id);
                                break;
                            }
                            "failed" => {
                                let reason =
                                    agent.exit_reason.as_deref().unwrap_or("unknown error");
                                return Err(ExitError::new(
                                    1,
                                    format!("Agent {} failed: {}", full_agent_id, reason),
                                )
                                .into());
                            }
                            _ => {
                                // Check exit_reason for agent-level terminals
                                match agent.exit_reason.as_deref() {
                                    Some("gone") => {
                                        return Err(ExitError::new(
                                            1,
                                            format!("Agent {} session gone", full_agent_id,),
                                        )
                                        .into());
                                    }
                                    Some(reason) if reason.starts_with("failed") => {
                                        return Err(ExitError::new(
                                            1,
                                            format!("Agent {} {}", full_agent_id, reason,),
                                        )
                                        .into());
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    None => {
                        // Agent no longer in the job's agent list — it finished
                        // and the job moved on. Treat as completed.
                        println!("Agent {} completed (no longer active)", full_agent_id);
                        break;
                    }
                }

                // Also check job-level terminal states as a fallback
                if p.step == "failed" {
                    let msg = p.error.as_deref().unwrap_or("unknown error");
                    return Err(ExitError::new(1, format!("Job {} failed: {}", p.name, msg)).into());
                }
                if p.step == "cancelled" {
                    return Err(ExitError::new(4, format!("Job {} was cancelled", p.name)).into());
                }
            }
        }
        match poller.tick().await {
            crate::poll::Tick::Ready => {}
            crate::poll::Tick::Timeout => {
                return Err(
                    ExitError::new(2, format!("Timeout waiting for agent {}", agent_id)).into(),
                );
            }
            crate::poll::Tick::Interrupted => {
                return Err(ExitError::new(130, String::new()).into());
            }
        }
    }

    Ok(())
}

/// Map a tool name from PreToolUse input to its corresponding PromptType.
/// Returns None for unrecognized tools.
fn prompt_type_for_tool(tool_name: Option<&str>) -> Option<PromptType> {
    match tool_name {
        Some("ExitPlanMode") | Some("EnterPlanMode") => Some(PromptType::PlanApproval),
        Some("AskUserQuestion") => Some(PromptType::Question),
        _ => None,
    }
}

async fn handle_pretooluse_hook(agent_id: &str, client: &DaemonClient) -> Result<()> {
    let mut input_json = String::new();
    io::stdin().read_to_string(&mut input_json)?;

    let input: PreToolUseInput = serde_json::from_str(&input_json).unwrap_or(PreToolUseInput {
        tool_name: None,
        tool_input: None,
    });

    let Some(prompt_type) = prompt_type_for_tool(input.tool_name.as_deref()) else {
        return Ok(());
    };

    // Extract question data from AskUserQuestion tool_input
    let question_data = if prompt_type == PromptType::Question {
        input
            .tool_input
            .as_ref()
            .and_then(|v| serde_json::from_value::<QuestionData>(v.clone()).ok())
    } else {
        None
    };

    let event = Event::AgentPrompt {
        agent_id: AgentId::new(agent_id),
        prompt_type,
        question_data,
    };
    client.emit_event(event).await?;

    Ok(())
}

async fn handle_notify_hook(agent_id: &str, client: &DaemonClient) -> Result<()> {
    let mut input_json = String::new();
    io::stdin().read_to_string(&mut input_json)?;

    let input: NotificationHookInput =
        serde_json::from_str(&input_json).unwrap_or(NotificationHookInput {
            notification_type: String::new(),
        });

    match input.notification_type.as_str() {
        "idle_prompt" => {
            let event = Event::AgentIdle {
                agent_id: AgentId::new(agent_id),
            };
            client.emit_event(event).await?;
        }
        "permission_prompt" => {
            let event = Event::AgentPrompt {
                agent_id: AgentId::new(agent_id),
                prompt_type: PromptType::Permission,
                question_data: None,
            };
            client.emit_event(event).await?;
        }
        _ => {
            // Ignore other notification types
        }
    }

    Ok(())
}

async fn handle_stop_hook(agent_id: &str, client: &DaemonClient) -> Result<()> {
    // Read JSON input from stdin (Claude Code sends this)
    let mut input_json = String::new();
    io::stdin().read_to_string(&mut input_json)?;

    let input: StopHookInput = serde_json::from_str(&input_json).unwrap_or(StopHookInput {
        stop_hook_active: false,
    });

    append_agent_log(
        agent_id,
        &format!("invoked, stop_hook_active={}", input.stop_hook_active),
    );

    // CRITICAL: Prevent infinite loops
    // If stop_hook_active is true, we're already in a stop hook chain - allow exit
    if input.stop_hook_active {
        append_agent_log(agent_id, "allowing exit, stop_hook_active=true");
        std::process::exit(0);
    }

    // Read on_stop config from agent state dir
    let on_stop = read_on_stop_config(agent_id);

    // Query daemon: has this agent signaled completion?
    let response = client.query_agent_signal(agent_id).await?;

    if response.signaled {
        // Agent has called `oj emit agent:signal` - allow stop
        append_agent_log(agent_id, "allowing exit, signaled=true");
        std::process::exit(0);
    }

    append_agent_log(
        agent_id,
        &format!("blocking exit, on_stop={}, signaled=false", on_stop),
    );

    match on_stop.as_str() {
        "idle" => {
            // Emit idle event, then block
            let event = Event::AgentIdle {
                agent_id: AgentId::new(agent_id),
            };
            let _ = client.emit_event(event).await;
            block_exit("Stop hook: on_idle handler invoked. Waiting for further instructions.");
        }
        "escalate" => {
            // Emit stop event for escalation, then block
            let event = Event::AgentStop {
                agent_id: AgentId::new(agent_id),
            };
            let _ = client.emit_event(event).await;
            block_exit("A human has been notified. Wait for instructions.");
        }
        _ => {
            // "signal" (default) — current behavior
            block_exit(&format!(
                "You must signal before stopping. Run one of:\n\
                 oj emit agent:signal --agent {} complete  — task is done\n\
                 oj emit agent:signal --agent {} escalate  — need human help\n\
                 oj emit agent:signal --agent {} continue  — still working on the task",
                agent_id, agent_id, agent_id
            ));
        }
    }
}

fn read_on_stop_config(agent_id: &str) -> String {
    let state_dir = get_state_dir();
    let config_path = state_dir.join("agents").join(agent_id).join("config.json");
    std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("on_stop")?.as_str().map(String::from))
        .unwrap_or_else(|| "signal".to_string())
}

fn block_exit(reason: &str) -> ! {
    let output = StopHookOutput {
        decision: "block".to_string(),
        reason: reason.to_string(),
    };
    let output_json = serde_json::to_string(&output).unwrap_or_default();
    let _ = io::stdout().write_all(output_json.as_bytes());
    let _ = io::stdout().flush();
    std::process::exit(0);
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
