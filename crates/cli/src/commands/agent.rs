// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent management commands

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

use crate::client::DaemonClient;
use crate::exit_error::ExitError;
use crate::output::{display_log, OutputFormat};

use super::pipeline::parse_duration;

#[derive(Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommand,
}

#[derive(Subcommand)]
pub enum AgentCommand {
    /// List agents across all pipelines
    List {
        /// Filter by pipeline ID (or prefix)
        #[arg(long)]
        pipeline: Option<String>,

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
    /// Send a message to a running agent
    Send {
        /// Agent ID or pipeline ID (or prefix)
        agent_id: String,
        /// Message to send
        message: String,
    },
    /// View agent activity log
    Logs {
        /// Pipeline ID (or prefix)
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
    /// Hook subcommands for Claude Code integration
    Hook {
        #[command(subcommand)]
        hook: HookCommand,
    },
}

#[derive(Subcommand)]
pub enum HookCommand {
    /// Stop hook handler - gates agent completion
    Stop {
        /// Agent ID to check
        agent_id: String,
    },
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

pub async fn handle(
    command: AgentCommand,
    client: &DaemonClient,
    format: OutputFormat,
) -> Result<()> {
    match command {
        AgentCommand::List {
            pipeline,
            status,
            limit,
            no_limit,
        } => {
            let agents = client
                .list_agents(pipeline.as_deref(), status.as_deref())
                .await?;

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
                        println!(
                            "{:<12} {:<12} {:<16} {:<10} {:>5} {:>5} {:>4}",
                            "AGENT_ID", "PIPELINE", "STEP", "STATUS", "READ", "WRITE", "CMDS"
                        );
                        for a in &agents {
                            println!(
                                "{:<12} {:<12} {:<16} {:<10} {:>5} {:>5} {:>4}",
                                truncate(&a.agent_id, 12),
                                truncate(&a.pipeline_id, 12),
                                truncate(&a.step_name, 16),
                                truncate(&a.status, 10),
                                a.files_read,
                                a.files_written,
                                a.commands_run,
                            );
                        }
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
        AgentCommand::Hook { hook } => match hook {
            HookCommand::Stop { agent_id } => {
                handle_stop_hook(&agent_id, client).await?;
            }
        },
    }

    Ok(())
}

/// Truncate a string to at most `max` characters for columnar display.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

/// Resolve the OJ state directory from environment or default.
fn get_state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OJ_STATE_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".local/state"))
                .unwrap_or_else(|_| PathBuf::from("."))
        })
        .join("oj")
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

/// Find an agent by ID (or prefix) across all pipelines.
/// Returns `(pipeline_id, agent_id)` on match, or None.
async fn find_agent(
    client: &DaemonClient,
    agent_id: &str,
) -> Result<Option<(String, String)>, anyhow::Error> {
    let pipelines = client.list_pipelines().await?;
    for summary in &pipelines {
        if let Some(detail) = client.get_pipeline(&summary.id).await? {
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
    let poll_ms = std::env::var("OJ_WAIT_POLL_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1000);
    let poll_interval = Duration::from_millis(poll_ms);
    let start = Instant::now();

    // Resolve agent to a pipeline on first iteration; re-scan if not found yet
    let mut resolved_pipeline_id: Option<String> = None;
    let mut resolved_agent_id: Option<String> = None;

    loop {
        // If we haven't found the agent yet, search for it
        if resolved_pipeline_id.is_none() {
            if let Some((pid, aid)) = find_agent(client, agent_id).await? {
                resolved_pipeline_id = Some(pid);
                resolved_agent_id = Some(aid);
            }
        }

        let pipeline_id = match &resolved_pipeline_id {
            Some(id) => id.clone(),
            None => {
                // Agent not found yet; check timeout before retrying
                if let Some(t) = timeout_dur {
                    if start.elapsed() >= t {
                        return Err(ExitError::new(
                            2,
                            format!("Timeout waiting for agent {}", agent_id),
                        )
                        .into());
                    }
                }
                // On first poll with no match, give a grace period for the agent to appear
                if start.elapsed() > Duration::from_secs(10) {
                    return Err(ExitError::new(3, format!("Agent not found: {}", agent_id)).into());
                }
                tokio::time::sleep(poll_interval).await;
                continue;
            }
        };

        let full_agent_id = resolved_agent_id.as_deref().unwrap_or(agent_id);

        let detail = client.get_pipeline(&pipeline_id).await?;
        match detail {
            None => {
                return Err(
                    ExitError::new(3, format!("Pipeline {} disappeared", pipeline_id)).into(),
                );
            }
            Some(p) => {
                // Find our specific agent in the pipeline
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
                        // Agent no longer in the pipeline's agent list — it finished
                        // and the pipeline moved on. Treat as completed.
                        println!("Agent {} completed (no longer active)", full_agent_id);
                        break;
                    }
                }

                // Also check pipeline-level terminal states as a fallback
                if p.step == "failed" {
                    let msg = p.error.as_deref().unwrap_or("unknown error");
                    return Err(
                        ExitError::new(1, format!("Pipeline {} failed: {}", p.name, msg)).into(),
                    );
                }
                if p.step == "cancelled" {
                    return Err(
                        ExitError::new(4, format!("Pipeline {} was cancelled", p.name)).into(),
                    );
                }
            }
        }
        if let Some(t) = timeout_dur {
            if start.elapsed() >= t {
                return Err(
                    ExitError::new(2, format!("Timeout waiting for agent {}", agent_id)).into(),
                );
            }
        }
        tokio::time::sleep(poll_interval).await;
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

    // Query daemon: has this agent signaled completion?
    let response = client.query_agent_signal(agent_id).await?;

    if response.signaled {
        // Agent has called `oj emit agent:signal` - allow stop
        append_agent_log(agent_id, "allowing exit, signaled=true");
        std::process::exit(0);
    }

    // Agent has NOT signaled - block and instruct
    append_agent_log(agent_id, "blocking exit, signaled=false");
    let output = StopHookOutput {
        decision: "block".to_string(),
        reason: format!(
            "You must explicitly signal completion before stopping. \
             Run: oj emit agent:signal --agent {} '<json>' \
             where <json> is {{\"action\": \"complete\"}} or {{\"action\": \"escalate\", \"message\": \"...\"}}",
            agent_id
        ),
    };

    let output_json = serde_json::to_string(&output)?;
    io::stdout().write_all(output_json.as_bytes())?;
    io::stdout().flush()?;

    std::process::exit(0);
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
