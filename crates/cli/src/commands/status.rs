// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj status` — cross-project overview dashboard.

use std::fmt::Write;

use anyhow::Result;

use crate::client::DaemonClient;
use crate::output::OutputFormat;

pub async fn handle(format: OutputFormat) -> Result<()> {
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
        OutputFormat::Text => format_text(uptime_secs, &namespaces),
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
        OutputFormat::Text => println!("oj daemon: not running"),
        OutputFormat::Json => println!(r#"{{ "status": "not_running" }}"#),
    }
    Ok(())
}

fn format_text(uptime_secs: u64, namespaces: &[oj_daemon::NamespaceStatus]) {
    let mut out = String::new();

    // Header line with uptime and global counts
    let uptime = format_duration(uptime_secs);
    let total_active: usize = namespaces.iter().map(|ns| ns.active_pipelines.len()).sum();
    let total_escalated: usize = namespaces
        .iter()
        .map(|ns| ns.escalated_pipelines.len())
        .sum();

    let _ = write!(out, "oj daemon: up {}", uptime);
    if total_active > 0 {
        let _ = write!(
            out,
            " | {} active pipeline{}",
            total_active,
            if total_active == 1 { "" } else { "s" }
        );
    }
    if total_escalated > 0 {
        let _ = write!(out, " | {} escalated", total_escalated);
    }
    out.push('\n');

    if namespaces.is_empty() {
        print!("{}", out);
        return;
    }

    for ns in namespaces {
        let label = if ns.namespace.is_empty() {
            "(default)"
        } else {
            &ns.namespace
        };

        // Check if this namespace has any content to show
        let has_content = !ns.active_pipelines.is_empty()
            || !ns.escalated_pipelines.is_empty()
            || !ns.workers.is_empty()
            || !ns.queues.is_empty()
            || !ns.active_agents.is_empty();

        if !has_content {
            continue;
        }

        // Namespace header
        let _ = write!(out, "\n── {} ", label);
        let pad = 48usize.saturating_sub(label.len() + 4);
        for _ in 0..pad {
            out.push('─');
        }
        out.push('\n');

        // Active pipelines
        if !ns.active_pipelines.is_empty() {
            let _ = writeln!(out, "  Pipelines ({} active):", ns.active_pipelines.len());
            for p in &ns.active_pipelines {
                let short_id = truncate_id(&p.id, 12);
                let elapsed = format_duration_ms(p.elapsed_ms);
                let _ = writeln!(
                    out,
                    "    {}  {}  {}  {}  {}",
                    short_id, p.name, p.step, p.step_status, elapsed,
                );
            }
            out.push('\n');
        }

        // Escalated pipelines
        if !ns.escalated_pipelines.is_empty() {
            let _ = writeln!(out, "  Escalated ({}):", ns.escalated_pipelines.len());
            for p in &ns.escalated_pipelines {
                let short_id = truncate_id(&p.id, 12);
                let elapsed = format_duration_ms(p.elapsed_ms);
                let _ = writeln!(
                    out,
                    "    ⚠ {}  {}  {}  {}  {}",
                    short_id, p.name, p.step, p.step_status, elapsed,
                );
                if let Some(ref reason) = p.waiting_reason {
                    let _ = writeln!(out, "      → {}", reason);
                }
            }
            out.push('\n');
        }

        // Workers
        if !ns.workers.is_empty() {
            let _ = writeln!(out, "  Workers:");
            for w in &ns.workers {
                let indicator = if w.status == "running" { "●" } else { "○" };
                let _ = writeln!(
                    out,
                    "    {}  {} {}  {}/{} active",
                    w.name, indicator, w.status, w.active, w.concurrency,
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
            let _ = writeln!(out, "  Queues:");
            for q in &non_empty_queues {
                let _ = write!(
                    out,
                    "    {}  {} pending, {} active",
                    q.name, q.pending, q.active,
                );
                if q.dead > 0 {
                    let _ = write!(out, ", {} dead", q.dead);
                }
                out.push('\n');
            }
            out.push('\n');
        }

        // Active agents
        if !ns.active_agents.is_empty() {
            let _ = writeln!(out, "  Agents ({} running):", ns.active_agents.len());
            for a in &ns.active_agents {
                let _ = writeln!(
                    out,
                    "    {}/{}  {}  {}",
                    a.pipeline_name, a.step_name, a.agent_id, a.status,
                );
            }
            out.push('\n');
        }
    }

    print!("{}", out);
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
