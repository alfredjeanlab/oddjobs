// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Display handlers for agent commands: List, Show, Peek, Attach, Send, Logs, Prune, Resume.

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;

use anyhow::Result;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::client::DaemonClient;
use crate::color;
use crate::output::{
    apply_limit, display_log, filter_by_project, format_or_json, handle_list_with_limit,
    poll_log_follow, print_capture_frame, print_prune_results, OutputFormat,
};
use crate::table::{Column, Table};
use oj_core::OwnerId;

pub(super) async fn handle_list(
    client: &DaemonClient,
    project_filter: Option<&str>,
    format: OutputFormat,
    job: Option<String>,
    status: Option<String>,
    limit: usize,
    no_limit: bool,
) -> Result<()> {
    let mut agents = client.list_agents(job.as_deref(), status.as_deref()).await?;

    // Filter by explicit --project flag (OJ_PROJECT is NOT used for filtering)
    filter_by_project(&mut agents, project_filter, |a| &a.project);
    let truncation = apply_limit(&mut agents, limit, no_limit);

    handle_list_with_limit(format, &agents, "No agents found", truncation, |items, out| {
        let cols = vec![
            Column::muted("ID").with_max(8),
            Column::left("KIND"),
            Column::left("PROJECT"),
            Column::left("JOB").with_max(8),
            Column::left("STEP"),
            Column::status("STATUS"),
            Column::right("READ"),
            Column::right("WRITE"),
            Column::right("CMDS"),
        ];
        let mut table = Table::new(cols);

        for a in items {
            let name = a.agent_name.as_deref().unwrap_or("-").to_string();
            let ns = if a.project.is_empty() { "-".to_string() } else { a.project.clone() };
            let job_col =
                a.owner.as_job().map(|id| id.to_string()).unwrap_or_else(|| "-".to_string());
            let step_col =
                if a.step_name.is_empty() { "-".to_string() } else { a.step_name.clone() };
            table.row(vec![
                a.agent_id.to_string(),
                name,
                ns,
                job_col,
                step_col,
                a.status.clone(),
                a.files_read.to_string(),
                a.files_written.to_string(),
                a.commands_run.to_string(),
            ]);
        }
        table.render(out);
    })?;
    Ok(())
}

pub(super) async fn handle_show(
    client: &DaemonClient,
    format: OutputFormat,
    id: &str,
) -> Result<()> {
    let agent = client.get_agent(id).await?;
    format_or_json(format, &agent, || {
        if let Some(a) = &agent {
            println!("{} {}", color::header("Agent:"), a.agent_id);
            println!("  {} {}", color::context("Name:"), a.agent_name.as_deref().unwrap_or("-"));
            if let OwnerId::Crew(id) = &a.owner {
                println!("  {} {}", color::context("Crew:"), id);
            }
            if !a.project.is_empty() {
                println!("  {} {}", color::context("Project:"), a.project);
            }
            match &a.owner {
                OwnerId::Job(id) => {
                    println!("  {} {} ({})", color::context("Job:"), id, a.job_name);
                    println!("  {} {}", color::context("Step:"), a.step_name);
                }
                OwnerId::Crew(_) => {
                    println!("  {} standalone", color::context("Source:"));
                }
            }
            println!("  {} {}", color::context("Status:"), color::status(&a.status));

            println!();
            println!("  {}", color::header("Activity:"));
            println!("    Files read: {}", a.files_read);
            println!("    Files written: {}", a.files_written);
            println!("    Commands run: {}", a.commands_run);

            println!();
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
    })?;
    Ok(())
}

pub(super) async fn handle_peek(client: &DaemonClient, id: &str) -> Result<()> {
    let agent =
        client.get_agent(id).await?.ok_or_else(|| anyhow::anyhow!("Agent not found: {}", id))?;

    let short_id = agent.agent_id.short(8);

    // Try saved terminal capture
    if let Some(content) = try_read_agent_capture(&agent.agent_id) {
        print_capture_frame(short_id, &content);
        return Ok(());
    }

    let is_terminal =
        agent.status == "completed" || agent.status == "failed" || agent.status == "cancelled";

    if is_terminal {
        println!("Agent {} is {}. No active agent.", short_id, agent.status);
    } else {
        println!("No capture available for agent {} (status: {})", short_id, agent.status);
    }
    println!();
    println!("Try:");
    println!("    oj agent logs {}", short_id);
    println!("    oj agent show {}", short_id);
    Ok(())
}

pub(super) async fn handle_attach(client: &DaemonClient, id: &str) -> Result<()> {
    match client.open_attach(id).await? {
        crate::client::AttachResult::Local { socket_path, .. } => {
            crate::daemon_process::coop_attach_socket(&socket_path)?;
        }
        crate::client::AttachResult::Remote { reader, writer, .. } => {
            run_attach_proxy(reader, writer).await?;
        }
    }
    Ok(())
}

pub(super) async fn handle_kill(client: &DaemonClient, id: &str) -> Result<()> {
    client.agent_kill(id).await?;
    println!("Killed agent {}", id);
    Ok(())
}

pub(super) async fn handle_suspend(client: &DaemonClient, id: &str) -> Result<()> {
    // Resolve agent to its owning job
    let agent =
        client.get_agent(id).await?.ok_or_else(|| anyhow::anyhow!("agent not found: {}", id))?;
    let job_id =
        agent.owner.as_job().ok_or_else(|| anyhow::anyhow!("agent {} has no owning job", id))?;
    let result = client.job_suspend(std::slice::from_ref(&job_id.to_string())).await?;
    for jid in &result.suspended {
        println!("Suspended job {} (via agent {})", jid, id);
    }
    for jid in &result.already_terminal {
        println!("Job {} was already terminal", jid);
    }
    for jid in &result.not_found {
        eprintln!("Job not found: {}", jid);
    }
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
    let (log_path, content, _steps, offset) = client.get_agent_logs(id, step, limit, 0).await?;
    if let Some(off) = display_log(&log_path, &content, follow, offset, format, "agent", id).await?
    {
        let id = id.to_string();
        let step = step.map(|s| s.to_string());
        poll_log_follow(off, |o| {
            let id = id.clone();
            let step = step.clone();
            async move {
                let (_, c, _, new_off) = client.get_agent_logs(&id, step.as_deref(), 0, o).await?;
                Ok((c, new_off))
            }
        })
        .await?;
    }
    Ok(())
}

pub(super) async fn handle_prune(
    client: &DaemonClient,
    format: OutputFormat,
    all: bool,
    dry_run: bool,
) -> Result<()> {
    let (pruned, skipped) = client.agent_prune(all, dry_run).await?;

    print_prune_results(&pruned, skipped, dry_run, format, "agent", "job(s) skipped", |entry| {
        match &entry.owner {
            OwnerId::Crew(_) => {
                format!("agent {} ({})", entry.agent_id.short(8), entry.step_name)
            }
            OwnerId::Job(id) => {
                format!("agent {} ({}, {})", entry.agent_id.short(8), id.short(8), entry.step_name)
            }
        }
    })?;
    Ok(())
}

/// RAII guard that puts the terminal into raw mode and restores it on drop.
struct RawTerminalGuard {
    original: nix::sys::termios::Termios,
}

impl RawTerminalGuard {
    fn new() -> Result<Self> {
        let stdin = std::io::stdin();
        let original = nix::sys::termios::tcgetattr(&stdin)?;
        let mut raw = original.clone();
        nix::sys::termios::cfmakeraw(&mut raw);
        nix::sys::termios::tcsetattr(&stdin, nix::sys::termios::SetArg::TCSANOW, &raw)?;
        Ok(Self { original })
    }
}

impl Drop for RawTerminalGuard {
    fn drop(&mut self) {
        let stdin = std::io::stdin();
        let _ = nix::sys::termios::tcsetattr(
            &stdin,
            nix::sys::termios::SetArg::TCSANOW,
            &self.original,
        );
    }
}

/// Run a bidirectional proxy between the local terminal and a remote agent
/// session through the daemon.
async fn run_attach_proxy(
    mut reader: Box<dyn AsyncRead + Unpin + Send>,
    mut writer: Box<dyn AsyncWrite + Unpin + Send>,
) -> Result<()> {
    let _guard = RawTerminalGuard::new()?;

    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let mut stdin_buf = [0u8; 4096];
    let mut remote_buf = [0u8; 4096];

    loop {
        tokio::select! {
            // stdin → remote
            result = stdin.read(&mut stdin_buf) => {
                match result {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if writer.write_all(&stdin_buf[..n]).await.is_err() {
                            break;
                        }
                        if writer.flush().await.is_err() {
                            break;
                        }
                    }
                }
            }
            // remote → stdout
            result = reader.read(&mut remote_buf) => {
                match result {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if stdout.write_all(&remote_buf[..n]).await.is_err() {
                            break;
                        }
                        if stdout.flush().await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Try to read a saved terminal capture for an agent. Returns `None` if
/// the state dir or capture file is unavailable.
pub(crate) fn try_read_agent_capture(agent_id: &oj_core::AgentId) -> Option<String> {
    let logs_dir = crate::env::state_dir().ok()?.join("logs");
    let path = oj_core::log_paths::agent_capture_path(&logs_dir, agent_id.as_str());
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

    let obj = serde_json::json!({
        "resumed": resumed,
        "skipped": skipped,
    });
    format_or_json(format, &obj, || {
        for aid in &resumed {
            println!("Resumed agent {}", oj_core::short(aid, 8));
        }
        for (aid, reason) in &skipped {
            println!("Skipped agent {}: {}", oj_core::short(aid, 8), reason);
        }
        if resumed.is_empty() && skipped.is_empty() {
            println!("No agents to resume");
        }
    })?;
    Ok(())
}
