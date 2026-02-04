// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj status` — cross-project overview dashboard.

use std::fmt::Write;
use std::io::IsTerminal;

use anyhow::Result;

use oj_core::ShortId;

use crate::client::DaemonClient;
use crate::color;
use crate::output::OutputFormat;

/// ANSI sequence: move cursor to top-left (home position).
/// Used instead of \x1B[2J (clear screen) to avoid pushing old content
/// into terminal scrollback.
const CURSOR_HOME: &str = "\x1B[H";

/// ANSI sequence: clear from cursor position to end of screen.
/// Removes leftover lines from a previous (longer) render.
const CLEAR_TO_END: &str = "\x1B[J";

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

    let is_tty = std::io::stdout().is_terminal();

    loop {
        handle_watch_frame(format, &args.interval, is_tty).await?;
        {
            use std::io::Write as _;
            std::io::stdout().flush()?;
        }
        tokio::time::sleep(interval).await;
    }
}

async fn handle_watch_frame(format: OutputFormat, interval: &str, is_tty: bool) -> Result<()> {
    let client = match DaemonClient::connect() {
        Ok(c) => c,
        Err(_) => {
            let content = format_not_running(format);
            print!("{}", render_frame(&content, is_tty));
            return Ok(());
        }
    };

    let (uptime_secs, namespaces) = match client.status_overview().await {
        Ok(data) => data,
        Err(crate::client::ClientError::DaemonNotRunning) => {
            let content = format_not_running(format);
            print!("{}", render_frame(&content, is_tty));
            return Ok(());
        }
        Err(crate::client::ClientError::Io(ref e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            let content = format_not_running(format);
            print!("{}", render_frame(&content, is_tty));
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    let content = match format {
        OutputFormat::Text => format_text(uptime_secs, &namespaces, Some(interval)),
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "uptime_secs": uptime_secs,
                "namespaces": namespaces,
            });
            format!("{}\n", serde_json::to_string_pretty(&obj)?)
        }
    };
    print!("{}", render_frame(&content, is_tty));

    Ok(())
}

/// Build one watch-mode frame.
///
/// When `is_tty` is true the frame is wrapped with ANSI cursor-home
/// before and clear-to-end after, so the terminal redraws in place
/// without polluting scrollback.  When false the content is returned
/// as-is (suitable for piped / redirected output).
fn render_frame(content: &str, is_tty: bool) -> String {
    if is_tty {
        format!("{CURSOR_HOME}{content}{CLEAR_TO_END}")
    } else {
        content.to_string()
    }
}

fn format_not_running(format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => format!("{} not running\n", color::header("oj daemon:")),
        OutputFormat::Json => r#"{ "status": "not_running" }"#.to_string() + "\n",
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
        // Note: queues need at least one non-zero count to be displayed
        let has_non_empty_queues = ns
            .queues
            .iter()
            .any(|q| q.pending > 0 || q.active > 0 || q.dead > 0);
        let has_content = !ns.active_pipelines.is_empty()
            || !ns.escalated_pipelines.is_empty()
            || !ns.orphaned_pipelines.is_empty()
            || !ns.workers.is_empty()
            || has_non_empty_queues
            || !ns.active_agents.is_empty();

        if !has_content {
            continue;
        }

        // Namespace header
        let label_colored = color::header(label);
        let _ = write!(out, "\n── {} ", label_colored);
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
            let rows: Vec<PipelineRow> = ns
                .active_pipelines
                .iter()
                .map(|p| PipelineRow {
                    prefix: "    ".to_string(),
                    id: p.id.short(8).to_string(),
                    name: friendly_name_label(&p.name, &p.kind, &p.id),
                    kind_step: format!("{}/{}", p.kind, p.step),
                    status: p.step_status.clone(),
                    suffix: format_duration_ms(p.elapsed_ms),
                    reason: None,
                })
                .collect();
            write_aligned_pipeline_rows(&mut out, &rows);
            out.push('\n');
        }

        // Escalated pipelines
        if !ns.escalated_pipelines.is_empty() {
            let _ = writeln!(
                out,
                "  {}",
                color::header(&format!("Escalated ({}):", ns.escalated_pipelines.len()))
            );
            let rows: Vec<PipelineRow> = ns
                .escalated_pipelines
                .iter()
                .map(|p| {
                    let source_label = p
                        .escalate_source
                        .as_deref()
                        .map(|s| format!("[{}]  ", s))
                        .unwrap_or_default();
                    let elapsed = format_duration_ms(p.elapsed_ms);
                    PipelineRow {
                        prefix: format!("    {} ", color::yellow("⚠")),
                        id: p.id.short(8).to_string(),
                        name: friendly_name_label(&p.name, &p.kind, &p.id),
                        kind_step: format!("{}/{}", p.kind, p.step),
                        status: p.step_status.clone(),
                        suffix: format!("{}{}", source_label, elapsed),
                        reason: p.waiting_reason.clone(),
                    }
                })
                .collect();
            write_aligned_pipeline_rows(&mut out, &rows);
            out.push('\n');
        }

        // Orphaned pipelines
        if !ns.orphaned_pipelines.is_empty() {
            let _ = writeln!(
                out,
                "  {}",
                color::header(&format!("Orphaned ({}):", ns.orphaned_pipelines.len()))
            );
            let rows: Vec<PipelineRow> = ns
                .orphaned_pipelines
                .iter()
                .map(|p| PipelineRow {
                    prefix: format!("    {} ", color::yellow("⚠")),
                    id: p.id.short(8).to_string(),
                    name: friendly_name_label(&p.name, &p.kind, &p.id),
                    kind_step: format!("{}/{}", p.kind, p.step),
                    status: "orphaned".to_string(),
                    suffix: format_duration_ms(p.elapsed_ms),
                    reason: None,
                })
                .collect();
            write_aligned_pipeline_rows(&mut out, &rows);
            let _ = writeln!(out, "    Run `oj daemon orphans` for recovery details");
            out.push('\n');
        }

        // Workers
        if !ns.workers.is_empty() {
            let _ = writeln!(out, "  {}", color::header("Workers:"));
            for w in &ns.workers {
                let _ = writeln!(
                    out,
                    "    {}  {}  {}/{} active",
                    w.name,
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
                    "    {} ({})  {}  {}",
                    a.agent_name,
                    a.command_name,
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

/// Returns the pipeline name when it is a meaningful friendly name,
/// or an empty string when it would be redundant (same as kind) or opaque (same as id).
fn friendly_name_label(name: &str, kind: &str, id: &str) -> String {
    // Hide name when it's empty, matches the kind, or matches the full/truncated ID.
    // When the name template produces an empty slug, pipeline_display_name() returns
    // just the nonce (first 8 chars of the ID), which would be redundant with the
    // truncated ID shown in the status output.
    let truncated_id = id.short(8);
    if name.is_empty() || name == kind || name == id || name == truncated_id {
        String::new()
    } else {
        name.to_string()
    }
}

/// A row of pipeline data for aligned rendering.
struct PipelineRow {
    prefix: String,
    id: String,
    name: String,
    kind_step: String,
    status: String,
    suffix: String,
    reason: Option<String>,
}

/// Render pipeline rows with aligned columns.
///
/// Columns: `{prefix}{id}  [{name}  ]{kind/step}  {status}  {suffix}`
/// The name column is omitted entirely when all names are empty.
fn write_aligned_pipeline_rows(out: &mut String, rows: &[PipelineRow]) {
    if rows.is_empty() {
        return;
    }

    let w_name = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
    let w_ks = rows.iter().map(|r| r.kind_step.len()).max().unwrap_or(0);
    let w_st = rows.iter().map(|r| r.status.len()).max().unwrap_or(0);

    for r in rows {
        let _ = write!(out, "{}{}", r.prefix, color::muted(&r.id));
        if w_name > 0 {
            let _ = write!(out, "  {:<w_name$}", r.name);
        }
        let _ = write!(
            out,
            "  {}",
            color::context(&format!("{:<w_ks$}", r.kind_step))
        );
        let _ = write!(out, "  {}", color::status(&format!("{:<w_st$}", r.status)));
        let _ = writeln!(out, "  {}", r.suffix);
        if let Some(ref reason) = r.reason {
            let _ = writeln!(out, "      → {}", truncate_reason(reason, 72));
        }
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
