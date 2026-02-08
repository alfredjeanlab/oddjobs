// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Display handlers for agent commands: List, Show, Peek, Attach, Send, Logs, Prune, Resume.

use anyhow::Result;

use oj_core::ShortId;

use crate::client::DaemonClient;
use crate::color;
use crate::output::{
    display_log, print_capture_frame, print_peek_frame, print_prune_results, should_use_color,
    OutputFormat,
};
use crate::table::{project_cell, should_show_project, Column, Table};

pub(super) async fn handle_list(
    client: &DaemonClient,
    project_filter: Option<&str>,
    format: OutputFormat,
    job: Option<String>,
    status: Option<String>,
    limit: usize,
    no_limit: bool,
) -> Result<()> {
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
    Ok(())
}

pub(super) async fn handle_show(
    client: &DaemonClient,
    format: OutputFormat,
    id: &str,
) -> Result<()> {
    let agent = client.get_agent(id).await?;

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
    Ok(())
}

pub(super) async fn handle_peek(client: &DaemonClient, id: &str) -> Result<()> {
    let agent = client
        .get_agent(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Agent not found: {}", id))?;

    let session_id = match agent.session_id {
        Some(ref id) => id.clone(),
        None => {
            // No session — try saved capture before giving up
            let short_id = agent.agent_id.short(8);
            if let Some(content) = try_read_agent_capture(&agent.agent_id) {
                print_capture_frame(short_id, &content);
                return Ok(());
            }
            anyhow::bail!("Agent has no active session");
        }
    };

    let with_color = should_use_color();
    match client.peek_session(&session_id, with_color).await {
        Ok(output) => {
            print_peek_frame(&session_id, &output);
        }
        Err(crate::client::ClientError::Rejected(msg)) if msg.starts_with("Session not found") => {
            let short_id = agent.agent_id.short(8);

            // Try reading saved terminal capture
            if let Some(content) = try_read_agent_capture(&agent.agent_id) {
                print_capture_frame(short_id, &content);
                return Ok(());
            }

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
    Ok(())
}

pub(super) async fn handle_attach(client: &DaemonClient, id: &str) -> Result<()> {
    let agent = client
        .get_agent(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Agent not found: {}", id))?;

    let session_id = agent
        .session_id
        .ok_or_else(|| anyhow::anyhow!("Agent has no active session"))?;

    super::super::session::attach(&session_id)?;
    Ok(())
}

pub(super) async fn handle_kill(client: &DaemonClient, id: &str) -> Result<()> {
    client.agent_kill(id).await?;
    println!("Killed agent {}", id);
    Ok(())
}

pub(super) async fn handle_send(
    client: &DaemonClient,
    agent_id: &str,
    message: &str,
) -> Result<()> {
    client.agent_send(agent_id, message).await?;
    println!("Sent to agent {}", agent_id);
    Ok(())
}

pub(super) async fn handle_logs(
    client: &DaemonClient,
    format: OutputFormat,
    id: &str,
    step: Option<&str>,
    follow: bool,
    limit: usize,
) -> Result<()> {
    let (log_path, content, _steps) = client.get_agent_logs(id, step, limit).await?;
    display_log(&log_path, &content, follow, format, "agent", id).await?;
    Ok(())
}

pub(super) async fn handle_prune(
    client: &DaemonClient,
    format: OutputFormat,
    all: bool,
    dry_run: bool,
) -> Result<()> {
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
    Ok(())
}

/// Try to read a saved terminal capture for an agent. Returns `None` if
/// the state dir or capture file is unavailable.
pub(crate) fn try_read_agent_capture(agent_id: &str) -> Option<String> {
    let logs_dir = crate::env::state_dir().ok()?.join("logs");
    let path = oj_engine::log_paths::agent_capture_path(&logs_dir, agent_id);
    std::fs::read_to_string(path).ok()
}

pub(super) async fn handle_resume(
    client: &DaemonClient,
    format: OutputFormat,
    id: Option<String>,
    kill: bool,
    all: bool,
) -> Result<()> {
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
    Ok(())
}
