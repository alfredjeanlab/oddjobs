// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj status` — cross-project overview dashboard.

use std::fmt::Write;

use anyhow::Result;

use crate::client::DaemonClient;
use crate::color;
use crate::output::OutputFormat;

#[derive(clap::Args)]
pub struct StatusArgs {
    /// Re-run status display in a loop (Ctrl+C to exit)
    #[arg(long)]
    pub watch: bool,

    /// Refresh interval for --watch mode (e.g. 2s, 10s)
    #[arg(long, default_value = "5s")]
    pub interval: String,
}

pub async fn handle(args: StatusArgs, format: OutputFormat) -> Result<()> {
    if !args.watch {
        return handle_once(format, None).await;
    }

    let interval = crate::commands::pipeline::parse_duration(&args.interval)?;
    if interval.is_zero() {
        anyhow::bail!("duration must be > 0");
    }

    loop {
        print!("\x1B[2J\x1B[H");
        handle_once(format, Some(&args.interval)).await?;
        tokio::time::sleep(interval).await;
    }
}

async fn handle_once(format: OutputFormat, watch_interval: Option<&str>) -> Result<()> {
    let client = match DaemonClient::connect() {
        Ok(c) => c,
        Err(_) => {
            return handle_not_running(format);
        }
    };

    let (uptime_secs, namespaces) = match client.status_overview().await {
        Ok(data) => data,
        Err(crate::client::ClientError::DaemonNotRunning) => {
            return handle_not_running(format);
        }
        Err(crate::client::ClientError::Io(ref e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            return handle_not_running(format);
        }
        Err(e) => return Err(e.into()),
    };

    match format {
        OutputFormat::Text => print!("{}", format_text(uptime_secs, &namespaces, watch_interval)),
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "uptime_secs": uptime_secs,
                "namespaces": namespaces,
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
    }

    Ok(())
}

fn handle_not_running(format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            println!("{} not running", color::header("oj daemon:"));
        }
        OutputFormat::Json => println!(r#"{{ "status": "not_running" }}"#),
    }
    Ok(())
}

fn format_text(
    uptime_secs: u64,
    namespaces: &[oj_daemon::NamespaceStatus],
    watch_interval: Option<&str>,
) -> String {
    let mut out = String::new();

    // Header line with uptime and global counts
    let uptime = format_duration(uptime_secs);
    let total_active: usize = namespaces.iter().map(|ns| ns.active_pipelines.len()).sum();
    let total_escalated: usize = namespaces
        .iter()
        .map(|ns| ns.escalated_pipelines.len())
        .sum();

    let _ = write!(
        out,
        "{} {} {}",
        color::header("oj daemon:"),
        color::status("running"),
        uptime
    );
    if let Some(interval) = watch_interval {
        let _ = write!(out, " | every {}", interval);
    }
    if total_active > 0 {
        let _ = write!(
            out,
            " | {} active pipeline{}",
            total_active,
            if total_active == 1 { "" } else { "s" }
        );
    }
    if total_escalated > 0 {
        let _ = write!(out, " | {} {}", total_escalated, color::status("escalated"));
    }
    let total_orphaned: usize = namespaces
        .iter()
        .map(|ns| ns.orphaned_pipelines.len())
        .sum();
    if total_orphaned > 0 {
        let _ = write!(out, " | {} {}", total_orphaned, color::status("orphaned"));
    }
    out.push('\n');

    if namespaces.is_empty() {
        return out;
    }

    for ns in namespaces {
        let label = if ns.namespace.is_empty() {
            "(no project)"
        } else {
            &ns.namespace
        };

        // Check if this namespace has any content to show
        let has_content = !ns.active_pipelines.is_empty()
            || !ns.escalated_pipelines.is_empty()
            || !ns.orphaned_pipelines.is_empty()
            || !ns.workers.is_empty()
            || !ns.queues.is_empty()
            || !ns.active_agents.is_empty();

        if !has_content {
            continue;
        }

        // Namespace header
        let label_colored = color::header(&format!("── {} ", label));
        let _ = write!(out, "\n{}", label_colored);
        let pad = 48usize.saturating_sub(label.len() + 4);
        for _ in 0..pad {
            out.push('─');
        }
        out.push('\n');

        // Active pipelines
        if !ns.active_pipelines.is_empty() {
            let _ = writeln!(
                out,
                "  {}",
                color::header(&format!(
                    "Pipelines ({} active):",
                    ns.active_pipelines.len()
                ))
            );
            for p in &ns.active_pipelines {
                let short_id = truncate_id(&p.id, 8);
                let elapsed = format_duration_ms(p.elapsed_ms);
                let friendly = friendly_name_label(&p.name, &p.kind, &p.id);
                let _ = writeln!(
                    out,
                    "    {}  {}{}  {}  {}  {}",
                    color::muted(short_id),
                    p.kind,
                    friendly,
                    p.step,
                    color::status(&p.step_status),
                    elapsed,
                );
            }
            out.push('\n');
        }

        // Escalated pipelines
        if !ns.escalated_pipelines.is_empty() {
            let _ = writeln!(
                out,
                "  {}",
                color::header(&format!("Escalated ({}):", ns.escalated_pipelines.len()))
            );
            for p in &ns.escalated_pipelines {
                let short_id = truncate_id(&p.id, 8);
                let elapsed = format_duration_ms(p.elapsed_ms);
                let friendly = friendly_name_label(&p.name, &p.kind, &p.id);
                let _ = writeln!(
                    out,
                    "    {} {}  {}{}  {}  {}  {}",
                    color::yellow("⚠"),
                    color::muted(short_id),
                    p.kind,
                    friendly,
                    p.step,
                    color::status(&p.step_status),
                    elapsed,
                );
                if let Some(ref reason) = p.waiting_reason {
                    let _ = writeln!(out, "      → {}", truncate_reason(reason, 72));
                }
            }
            out.push('\n');
        }

        // Orphaned pipelines
        if !ns.orphaned_pipelines.is_empty() {
            let _ = writeln!(
                out,
                "  {}",
                color::header(&format!("Orphaned ({}):", ns.orphaned_pipelines.len()))
            );
            for p in &ns.orphaned_pipelines {
                let short_id = truncate_id(&p.id, 8);
                let elapsed = format_duration_ms(p.elapsed_ms);
                let friendly = friendly_name_label(&p.name, &p.kind, &p.id);
                let _ = writeln!(
                    out,
                    "    {} {}  {}{}  {}  {}  {}",
                    color::yellow("⚠"),
                    color::muted(short_id),
                    p.kind,
                    friendly,
                    p.step,
                    color::status("orphaned"),
                    elapsed,
                );
            }
            let _ = writeln!(out, "    Run `oj daemon orphans` for recovery details");
            out.push('\n');
        }

        // Workers
        if !ns.workers.is_empty() {
            let _ = writeln!(out, "  {}", color::header("Workers:"));
            for w in &ns.workers {
                let indicator = if w.status == "running" {
                    color::green("●")
                } else {
                    color::muted("○")
                };
                let _ = writeln!(
                    out,
                    "    {}  {} {}  {}/{} active",
                    w.name,
                    indicator,
                    color::status(&w.status),
                    w.active,
                    w.concurrency,
                );
            }
            out.push('\n');
        }

        // Queues
        let non_empty_queues: Vec<_> = ns
            .queues
            .iter()
            .filter(|q| q.pending > 0 || q.active > 0 || q.dead > 0)
            .collect();
        if !non_empty_queues.is_empty() {
            let _ = writeln!(out, "  {}", color::header("Queues:"));
            for q in &non_empty_queues {
                let _ = write!(
                    out,
                    "    {}  {} pending, {} active",
                    q.name, q.pending, q.active,
                );
                if q.dead > 0 {
                    let _ = write!(out, ", {} {}", q.dead, color::status("dead"));
                }
                out.push('\n');
            }
            out.push('\n');
        }

        // Active agents
        if !ns.active_agents.is_empty() {
            let _ = writeln!(
                out,
                "  {}",
                color::header(&format!("Agents ({} running):", ns.active_agents.len()))
            );
            for a in &ns.active_agents {
                let _ = writeln!(
                    out,
                    "    {}/{}  {}  {}",
                    a.pipeline_name,
                    a.step_name,
                    color::muted(&a.agent_id),
                    color::status(&a.status),
                );
            }
            out.push('\n');
        }
    }

    out
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m > 0 {
            format!("{}h{}m", h, m)
        } else {
            format!("{}h", h)
        }
    } else {
        format!("{}d", secs / 86400)
    }
}

fn format_duration_ms(ms: u64) -> String {
    format_duration(ms / 1000)
}

fn truncate_id(id: &str, max_len: usize) -> &str {
    if id.len() <= max_len {
        id
    } else {
        &id[..max_len]
    }
}

/// Returns ` (name)` when the pipeline name is a meaningful friendly name,
/// or an empty string when it would be redundant (same as kind) or opaque (same as id).
fn friendly_name_label(name: &str, kind: &str, id: &str) -> String {
    if name.is_empty() || name == kind || name == id {
        String::new()
    } else {
        format!(" ({})", name)
    }
}

fn truncate_reason(reason: &str, max_len: usize) -> String {
    // Take only the first line, then truncate to max_len
    let first_line = reason.lines().next().unwrap_or(reason);
    let multiline = reason.contains('\n');
    if first_line.len() <= max_len && !multiline {
        first_line.to_string()
    } else {
        let limit = max_len.saturating_sub(3);
        let truncated = if first_line.len() > limit {
            &first_line[..limit]
        } else {
            first_line
        };
        format!("{}...", truncated)
    }
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
