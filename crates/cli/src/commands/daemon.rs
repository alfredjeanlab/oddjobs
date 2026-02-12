// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj daemon` - Daemon management commands

use crate::client::DaemonClient;
use crate::client_lifecycle::daemon_stop;
use crate::output::{display_log, format_or_json, handle_list, OutputFormat};
use crate::table::{Column, Table};
use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Command;

#[derive(Args)]
pub struct DaemonArgs {
    /// Print daemon version
    #[arg(short = 'v', long = "version")]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<DaemonCommand>,
}

#[derive(Subcommand)]
pub enum DaemonCommand {
    /// Start the daemon (foreground or background)
    Start {
        /// Run in foreground (useful for debugging)
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the daemon
    Stop {
        /// Kill all active sessions (agents, shells) before stopping
        #[arg(long)]
        kill: bool,
    },
    /// Check daemon status
    Status,
    /// Stop and restart the daemon
    Restart {
        /// Kill all active sessions (agents, shells) before restarting
        #[arg(long)]
        kill: bool,
    },
    /// View daemon logs
    Logs {
        /// Number of recent lines to show (default: 200)
        #[arg(short = 'n', long, default_value = "200")]
        limit: usize,
        /// Show all lines (no limit)
        #[arg(long, conflicts_with = "limit")]
        no_limit: bool,
        /// Follow log output
        #[arg(long, short)]
        follow: bool,
    },
    /// List orphaned jobs detected at startup
    Orphans {
        /// Dismiss an orphaned job by ID (or prefix)
        #[arg(long)]
        dismiss: Option<String>,
    },
}

pub async fn daemon(args: DaemonArgs, format: OutputFormat) -> Result<()> {
    if args.version {
        return version(format).await;
    }

    match args.command {
        Some(DaemonCommand::Start { foreground }) => start(foreground).await,
        Some(DaemonCommand::Stop { kill }) => stop(kill).await,
        Some(DaemonCommand::Restart { kill }) => restart(kill).await,
        Some(DaemonCommand::Status) => status(format).await,
        Some(DaemonCommand::Logs { limit, no_limit, follow }) => {
            logs(limit, no_limit, follow, format).await
        }
        Some(DaemonCommand::Orphans { dismiss: Some(id) }) => dismiss_orphan(id, format).await,
        Some(DaemonCommand::Orphans { dismiss: None }) => orphans(format).await,
        None => {
            // No subcommand — show colorized help
            let cmd = crate::find_subcommand(crate::cli_command(), &["daemon"]);
            crate::help::print_help(cmd);
            Ok(())
        }
    }
}

async fn version(format: OutputFormat) -> Result<()> {
    let client = match DaemonClient::connect() {
        Ok(c) => c,
        Err(_) => return print_not_running(format),
    };

    let version = match client.hello().await {
        Ok(v) => v,
        Err(e) if e.is_not_running() => return print_not_running(format),
        Err(_) => "unknown".to_string(),
    };

    let obj = serde_json::json!({ "version": version });
    format_or_json(format, &obj, || println!("ojd {}", version))
}

async fn start(foreground: bool) -> Result<()> {
    if foreground {
        // Run daemon in foreground - spawn and wait
        let ojd_path = find_ojd_binary()?;
        let status = Command::new(&ojd_path).status()?;
        if !status.success() {
            return Err(anyhow!("Daemon exited with status: {}", status));
        }
        return Ok(());
    }

    // Check if already running
    if let Ok(client) = DaemonClient::connect() {
        if let Ok((uptime, _, _)) = client.status().await {
            println!("Daemon already running (uptime: {}s)", uptime);
            return Ok(());
        }
    }

    // Start in background and verify it started
    match DaemonClient::connect_or_start() {
        Ok(_client) => {
            println!("Daemon started");
            Ok(())
        }
        Err(e) => Err(anyhow!("{}", e)),
    }
}

async fn stop(kill: bool) -> Result<()> {
    match daemon_stop(kill).await {
        Ok(true) => {
            println!("Daemon stopped");
            Ok(())
        }
        Ok(false) => {
            println!("Daemon not running");
            Ok(())
        }
        Err(e) => Err(anyhow!("Failed to stop daemon: {}", e)),
    }
}

async fn restart(kill: bool) -> Result<()> {
    // Stop the daemon if running (ignore "not running" case)
    let was_running =
        daemon_stop(kill).await.map_err(|e| anyhow!("Failed to stop daemon: {}", e))?;

    if was_running {
        // Brief wait for the process to fully exit and release the socket.
        // This is not a synchronization hack — it's a grace period for the OS
        // to release the Unix socket after the daemon process exits.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Start in background
    match DaemonClient::connect_or_start() {
        Ok(_client) => {
            println!("Daemon restarted");
            Ok(())
        }
        Err(e) => Err(anyhow!("{}", e)),
    }
}

async fn status(format: OutputFormat) -> Result<()> {
    let client = match DaemonClient::connect() {
        Ok(c) => c,
        Err(_) => return print_not_running(format),
    };

    // Handle connection errors (socket exists but daemon not running)
    let (uptime, jobs, orphan_count) = match client.status().await {
        Ok(result) => result,
        Err(e) if e.is_not_running() => return print_not_running(format),
        Err(e) => return Err(anyhow!("{}", e)),
    };
    let version = client.hello().await.unwrap_or_else(|_| "unknown".to_string());

    let obj = serde_json::json!({
        "status": "running",
        "version": version,
        "uptime_secs": uptime,
        "uptime": format_uptime(uptime),
        "jobs_active": jobs,
        "orphan_count": orphan_count,
    });
    format_or_json(format, &obj, || {
        let uptime_str = format_uptime(uptime);
        println!("Status: running");
        println!("Version: {}", version);
        println!("Uptime: {}", uptime_str);
        println!("Jobs: {} active", jobs);
        if orphan_count > 0 {
            println!();
            println!(
                "\u{26a0} {} orphaned job(s) detected (missing from WAL/snapshot)",
                orphan_count
            );
            println!("  Run `oj daemon orphans` for details");
        }
    })
}

async fn logs(limit: usize, no_limit: bool, follow: bool, format: OutputFormat) -> Result<()> {
    let log_path = get_log_path()?;

    if !log_path.exists() {
        let empty: Vec<String> = vec![];
        let obj = serde_json::json!({
            "log_path": log_path.to_string_lossy().into_owned(),
            "lines": empty,
        });
        return format_or_json(format, &obj, || {
            println!("No log file found at {}", log_path.display())
        });
    }

    // Read the last N lines (or all lines with --no-limit)
    let content = if no_limit {
        std::fs::read_to_string(&log_path)?
    } else {
        read_last_lines(&log_path, limit)?
    };
    display_log(&log_path, &content, follow, 0, format, "daemon", "log").await?;
    Ok(())
}

async fn orphans(format: OutputFormat) -> Result<()> {
    let client = DaemonClient::connect().map_err(|e| anyhow!("{}", e))?;
    let orphans = client.list_orphans().await.map_err(|e| anyhow!("{}", e))?;

    handle_list(format, &orphans, "No orphaned jobs detected.", |items, out| {
        let _ = writeln!(out, "Orphaned Jobs (not in recovered state):\n");
        let mut table = Table::new(vec![
            Column::muted("ID"),
            Column::left("PROJECT").with_max(12),
            Column::left("KIND").with_max(10),
            Column::left("NAME").with_max(20),
            Column::left("STEP").with_max(12),
            Column::status("STATUS"),
            Column::left("LAST UPDATED"),
        ]);
        for o in items {
            table.row(vec![
                oj_core::short(&o.job_id, 8).to_string(),
                o.project.clone(),
                o.kind.clone(),
                o.name.clone(),
                o.current_step.clone(),
                o.step_status.to_string(),
                o.updated_at.clone(),
            ]);
        }
        table.render(out);

        let _ = writeln!(out, "\nCommands (replace <id> with an orphan ID above):");
        let _ = writeln!(out, "  oj job peek <id>              # View agent output");
        let _ = writeln!(out, "  oj job attach <id>            # Attach to agent session");
        let _ = writeln!(out, "  oj job logs <id>              # View job log");
        let _ = writeln!(out, "  oj daemon orphans --dismiss <id>   # Dismiss after investigation");
        let _ = writeln!(out, "  oj job prune --orphans        # Dismiss all orphans");
    })
}

async fn dismiss_orphan(id: String, format: OutputFormat) -> Result<()> {
    let client = DaemonClient::connect().map_err(|e| anyhow!("{}", e))?;
    client.dismiss_orphan(&id).await.map_err(|e| anyhow!("{}", e))?;

    let obj = serde_json::json!({ "dismissed": &id });
    format_or_json(format, &obj, || println!("Orphan dismissed: {}", id))
}

fn print_not_running(format: OutputFormat) -> Result<()> {
    let obj = serde_json::json!({ "status": "not_running" });
    format_or_json(format, &obj, || println!("Daemon not running"))
}

fn read_last_lines(path: &std::path::Path, n: usize) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let lines: Vec<String> = BufReader::new(file).lines().collect::<std::io::Result<_>>()?;
    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].join("\n"))
}

fn format_uptime(secs: u64) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, mins, secs)
    } else if mins > 0 {
        format!("{}m {}s", mins, secs)
    } else {
        format!("{}s", secs)
    }
}

fn find_ojd_binary() -> Result<PathBuf> {
    let current_exe = std::env::current_exe().ok();

    // Only use CARGO_MANIFEST_DIR if the CLI itself is a debug build.
    // This prevents version mismatches when agents run in coop sessions that
    // inherit CARGO_MANIFEST_DIR from a dev environment but use release builds.
    let is_debug_build = current_exe
        .as_ref()
        .and_then(|p| p.to_str())
        .map(|s| s.contains("target/debug"))
        .unwrap_or(false);

    if is_debug_build {
        if let Some(manifest_dir) = crate::env::cargo_manifest_dir() {
            let dev_path = PathBuf::from(manifest_dir)
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("target/debug/ojd"));
            if let Some(path) = dev_path {
                if path.exists() {
                    return Ok(path);
                }
            }
        }
    }

    // Check current executable's directory
    if let Some(ref exe) = current_exe {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("ojd");
            if sibling.exists() {
                return Ok(sibling);
            }
        }
    }

    // Fall back to PATH lookup
    Ok(PathBuf::from("ojd"))
}

#[cfg(test)]
#[path = "daemon_tests.rs"]
mod tests;

fn get_log_path() -> Result<PathBuf> {
    let state_dir = crate::env::state_dir()
        .map_err(|e| anyhow!("could not determine state directory: {}", e))?;
    Ok(state_dir.join("daemon.log"))
}
