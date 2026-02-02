// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj daemon` - Daemon management commands

use crate::client::DaemonClient;
use crate::client_lifecycle::daemon_stop;
use crate::output::{display_log, OutputFormat};
use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use std::path::PathBuf;
use std::process::Command;

#[derive(Args)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub command: DaemonCommand,
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
        /// Number of lines to show
        #[arg(long, default_value = "100")]
        lines: usize,
        /// Follow log output
        #[arg(long, short)]
        follow: bool,
    },
}

pub async fn daemon(args: DaemonArgs, format: OutputFormat) -> Result<()> {
    match args.command {
        DaemonCommand::Start { foreground } => start(foreground).await,
        DaemonCommand::Stop { kill } => stop(kill).await,
        DaemonCommand::Restart { kill } => restart(kill).await,
        DaemonCommand::Status => status(format).await,
        DaemonCommand::Logs { lines, follow } => logs(lines, follow, format).await,
    }
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
    let was_running = daemon_stop(kill)
        .await
        .map_err(|e| anyhow!("Failed to stop daemon: {}", e))?;

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
    let not_running = || match format {
        OutputFormat::Text => {
            println!("Daemon not running");
            Ok(())
        }
        OutputFormat::Json => {
            println!(r#"{{ "status": "not_running" }}"#);
            Ok(())
        }
    };

    let client = match DaemonClient::connect() {
        Ok(c) => c,
        Err(_) => return not_running(),
    };

    // Handle connection errors (socket exists but daemon not running)
    let (uptime, pipelines, sessions) = match client.status().await {
        Ok(result) => result,
        Err(crate::client::ClientError::DaemonNotRunning) => return not_running(),
        Err(crate::client::ClientError::Io(ref e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            return not_running();
        }
        Err(e) => return Err(anyhow!("{}", e)),
    };
    let version = client
        .hello()
        .await
        .unwrap_or_else(|_| "unknown".to_string());

    match format {
        OutputFormat::Text => {
            let uptime_str = format_uptime(uptime);
            println!("Status: running");
            println!("Version: {}", version);
            println!("Uptime: {}", uptime_str);
            println!("Pipelines: {} active", pipelines);
            println!("Sessions: {} active", sessions);
        }
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "status": "running",
                "version": version,
                "uptime_secs": uptime,
                "uptime": format_uptime(uptime),
                "pipelines_active": pipelines,
                "sessions_active": sessions,
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
    }

    Ok(())
}

async fn logs(lines: usize, follow: bool, format: OutputFormat) -> Result<()> {
    let log_path = get_log_path()?;

    if !log_path.exists() {
        match format {
            OutputFormat::Text => println!("No log file found at {}", log_path.display()),
            OutputFormat::Json => {
                let obj = serde_json::json!({
                    "log_path": log_path.to_string_lossy(),
                    "lines": [],
                });
                println!("{}", serde_json::to_string_pretty(&obj)?);
            }
        }
        return Ok(());
    }

    // Read the last N lines
    let content = read_last_lines(&log_path, lines)?;
    display_log(&log_path, &content, follow, format, "daemon", "log").await
}

fn read_last_lines(path: &std::path::Path, n: usize) -> Result<String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path)?;
    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .collect::<std::io::Result<_>>()?;
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
    // This prevents version mismatches when agents run in tmux sessions that
    // inherit CARGO_MANIFEST_DIR from a dev environment but use release builds.
    let is_debug_build = current_exe
        .as_ref()
        .and_then(|p| p.to_str())
        .map(|s| s.contains("target/debug"))
        .unwrap_or(false);

    if is_debug_build {
        if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
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

fn get_log_path() -> Result<PathBuf> {
    // OJ_STATE_DIR takes priority (used by tests for isolation)
    if let Ok(dir) = std::env::var("OJ_STATE_DIR") {
        return Ok(PathBuf::from(dir).join("daemon.log"));
    }

    // Fall back to XDG_STATE_HOME/oj or ~/.local/state/oj
    let state_dir = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".local/state"))
                .unwrap_or_else(|_| PathBuf::from("."))
        })
        .join("oj");

    Ok(state_dir.join("daemon.log"))
}
