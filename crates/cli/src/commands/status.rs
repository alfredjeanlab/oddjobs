// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj status` — cross-project overview dashboard.

use std::fmt::Write;
use std::io::IsTerminal;
use std::io::Write as _;

use anyhow::Result;

use crate::client::DaemonClient;
use crate::color;
use crate::output::{format_or_json, OutputFormat};

/// ANSI sequence: move cursor to top-left (home position).
/// Used instead of \x1B[2J (clear screen) to avoid pushing old content
/// into terminal scrollback.
const CURSOR_HOME: &str = "\x1B[H";

/// ANSI sequence: clear from cursor position to end of screen.
/// Removes leftover lines from a previous (longer) render.
const CLEAR_TO_END: &str = "\x1B[J";

/// ANSI sequence: clear from cursor position to end of line.
/// Removes leftover characters from a previous (wider) render on the same line.
const CLEAR_TO_EOL: &str = "\x1B[K";

#[derive(clap::Args)]
pub struct StatusArgs {
    /// Re-run status display in a loop (Ctrl+C to exit)
    #[arg(long)]
    pub watch: bool,

    /// Refresh interval for --watch mode (e.g. 2s, 10s)
    #[arg(long, default_value = "5s")]
    pub interval: String,
}

pub async fn handle(
    args: StatusArgs,
    format: OutputFormat,
    project_filter: Option<&str>,
) -> Result<()> {
    if !args.watch {
        return handle_once(format, None, project_filter).await;
    }

    let interval = crate::commands::job::parse_duration(&args.interval)?;
    if interval.is_zero() {
        anyhow::bail!("duration must be > 0");
    }

    let is_tty = std::io::stdout().is_terminal();

    loop {
        handle_watch_frame(format, &args.interval, is_tty, project_filter).await?;
        std::io::stdout().flush()?;
        tokio::time::sleep(interval).await;
    }
}

async fn handle_watch_frame(
    format: OutputFormat,
    interval: &str,
    is_tty: bool,
    project_filter: Option<&str>,
) -> Result<()> {
    let (uptime_secs, mut namespaces, metrics_health) = match fetch_overview().await? {
        Some(data) => data,
        None => {
            print!("{}", render_frame(&format_not_running(format), is_tty));
            return Ok(());
        }
    };

    if let Some(proj) = project_filter {
        namespaces.retain(|ns| ns.project == proj);
    }
    let content = match format {
        OutputFormat::Text => {
            format_text(uptime_secs, &namespaces, Some(interval), metrics_health.as_ref())
        }
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "uptime_secs": uptime_secs,
                "namespaces": namespaces,
                "metrics_health": metrics_health,
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
/// without polluting scrollback.  Each line also gets a clear-to-EOL
/// sequence so that a shorter line does not leave remnants from the
/// previous (wider) frame.  When false the content is returned as-is
/// (suitable for piped / redirected output).
fn render_frame(content: &str, is_tty: bool) -> String {
    if is_tty {
        let cleared = content.replace('\n', &format!("{CLEAR_TO_EOL}\n"));
        format!("{CURSOR_HOME}{cleared}{CLEAR_TO_END}")
    } else {
        content.to_string()
    }
}

/// Connect to the daemon and fetch the status overview.
/// Returns `Ok(None)` when the daemon is not reachable.
async fn fetch_overview(
) -> Result<Option<(u64, Vec<oj_wire::ProjectStatus>, Option<oj_wire::MetricsHealthSummary>)>> {
    let client = match DaemonClient::connect() {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    match client.status_overview().await {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.is_not_running() => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn format_not_running(format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => format!("{} not running\n", color::header("oj daemon:")),
        OutputFormat::Json => "{ \"status\": \"not_running\" }\n".to_string(),
    }
}

async fn handle_once(
    format: OutputFormat,
    watch_interval: Option<&str>,
    project_filter: Option<&str>,
) -> Result<()> {
    let (uptime_secs, mut namespaces, metrics_health) = match fetch_overview().await? {
        Some(data) => data,
        None => {
            print!("{}", format_not_running(format));
            return Ok(());
        }
    };

    if let Some(proj) = project_filter {
        namespaces.retain(|ns| ns.project == proj);
    }
    let obj = serde_json::json!({
        "uptime_secs": uptime_secs,
        "namespaces": &namespaces,
        "metrics_health": &metrics_health,
    });
    format_or_json(format, &obj, || {
        print!("{}", format_text(uptime_secs, &namespaces, watch_interval, metrics_health.as_ref()))
    })
}

fn format_text(
    uptime_secs: u64,
    namespaces: &[oj_wire::ProjectStatus],
    watch_interval: Option<&str>,
    metrics_health: Option<&oj_wire::MetricsHealthSummary>,
) -> String {
    let mut out = String::new();

    // Header line with uptime and global counts
    let uptime = oj_core::format_elapsed(uptime_secs);
    let total_active: usize = namespaces.iter().map(|ns| ns.active_jobs.len()).sum();
    let total_escalated: usize = namespaces.iter().map(|ns| ns.escalated_jobs.len()).sum();

    let _ = write!(out, "{} {} {}", color::header("oj daemon:"), color::status("running"), uptime);
    if let Some(interval) = watch_interval {
        let _ = write!(out, " | every {}", interval);
    }
    if total_active > 0 {
        let _ = write!(
            out,
            " | {} active job{}",
            total_active,
            if total_active == 1 { "" } else { "s" }
        );
    }
    if total_escalated > 0 {
        let _ = write!(out, " | {} {}", total_escalated, color::status("escalated"));
    }
    let total_suspended: usize = namespaces.iter().map(|ns| ns.suspended_jobs.len()).sum();
    if total_suspended > 0 {
        let _ = write!(out, " | {} {}", total_suspended, color::status("suspended"));
    }
    let total_orphaned: usize = namespaces.iter().map(|ns| ns.orphaned_jobs.len()).sum();
    if total_orphaned > 0 {
        let _ = write!(out, " | {} {}", total_orphaned, color::status("orphaned"));
    }
    let total_pending_decisions: usize = namespaces.iter().map(|ns| ns.pending_decisions).sum();
    if total_pending_decisions > 0 {
        let _ = write!(
            out,
            " | {} decision{} pending",
            total_pending_decisions,
            if total_pending_decisions == 1 { "" } else { "s" }
        );
    }
    // Metrics health summary
    if let Some(mh) = metrics_health {
        if let Some(ref err) = mh.last_error {
            let short = if err.len() > 30 { format!("{}...", &err[..27]) } else { err.clone() };
            let _ = write!(out, " | metrics: {} ({})", color::status("error"), short);
        }
    }
    out.push('\n');

    if namespaces.is_empty() {
        return out;
    }

    for ns in namespaces {
        let label = if ns.project.is_empty() { "(no project)" } else { &ns.project };

        // Check if this project has any content to show
        // Note: queues need at least one non-zero count to be displayed
        let has_non_empty_queues =
            ns.queues.iter().any(|q| q.pending > 0 || q.active > 0 || q.dead > 0);
        let has_content = !ns.active_jobs.is_empty()
            || !ns.escalated_jobs.is_empty()
            || !ns.suspended_jobs.is_empty()
            || !ns.orphaned_jobs.is_empty()
            || !ns.workers.is_empty()
            || !ns.crons.is_empty()
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

        // Sort jobs by most recent activity (descending) and workers alphabetically
        let active_jobs = sorted_by_activity(&ns.active_jobs);
        let escalated_jobs = sorted_by_activity(&ns.escalated_jobs);
        let suspended_jobs = sorted_by_activity(&ns.suspended_jobs);
        let orphaned_jobs = sorted_by_activity(&ns.orphaned_jobs);
        let mut workers: Vec<&oj_wire::WorkerSummary> = ns.workers.iter().collect();
        workers.sort_by(|a, b| a.name.cmp(&b.name));

        // Active jobs
        if !active_jobs.is_empty() {
            let _ = writeln!(
                out,
                "  {}",
                color::header(&format!("Jobs ({} active):", active_jobs.len()))
            );
            let rows: Vec<JobRow> = active_jobs
                .iter()
                .map(|p| JobRow {
                    prefix: "    ".to_string(),
                    id: oj_core::short(&p.id, 8).to_string(),
                    name: friendly_name_label(&p.name, &p.kind, &p.id),
                    kind_step: format!("{}/{}", p.kind, p.step),
                    status: p.step_status.to_string(),
                    suffix: oj_core::format_elapsed_ms(p.elapsed_ms),
                    reason: None,
                })
                .collect();
            write_aligned_job_rows(&mut out, &rows);
            out.push('\n');
        }

        // Escalated jobs
        if !escalated_jobs.is_empty() {
            let _ = writeln!(
                out,
                "  {}",
                color::header(&format!("Escalated ({}):", escalated_jobs.len()))
            );
            let rows: Vec<JobRow> = escalated_jobs
                .iter()
                .map(|p| {
                    let source_label = p
                        .escalate_source
                        .as_deref()
                        .map(|s| format!("[{}]  ", s))
                        .unwrap_or_default();
                    let elapsed = oj_core::format_elapsed_ms(p.elapsed_ms);
                    JobRow {
                        prefix: format!("    {} ", color::yellow("⚠")),
                        id: oj_core::short(&p.id, 8).to_string(),
                        name: friendly_name_label(&p.name, &p.kind, &p.id),
                        kind_step: format!("{}/{}", p.kind, p.step),
                        status: p.step_status.to_string(),
                        suffix: format!("{}{}", source_label, elapsed),
                        reason: p.waiting_reason.clone(),
                    }
                })
                .collect();
            write_aligned_job_rows(&mut out, &rows);
            out.push('\n');
        }

        // Orphaned jobs
        if !orphaned_jobs.is_empty() {
            let _ = writeln!(
                out,
                "  {}",
                color::header(&format!("Orphaned ({}):", orphaned_jobs.len()))
            );
            let rows: Vec<JobRow> = orphaned_jobs
                .iter()
                .map(|p| JobRow {
                    prefix: format!("    {} ", color::yellow("⚠")),
                    id: oj_core::short(&p.id, 8).to_string(),
                    name: friendly_name_label(&p.name, &p.kind, &p.id),
                    kind_step: format!("{}/{}", p.kind, p.step),
                    status: "orphaned".to_string(),
                    suffix: oj_core::format_elapsed_ms(p.elapsed_ms),
                    reason: None,
                })
                .collect();
            write_aligned_job_rows(&mut out, &rows);
            let _ = writeln!(out, "    Run `oj daemon orphans` for recovery details");
            out.push('\n');
        }

        // Suspended jobs
        if !suspended_jobs.is_empty() {
            let _ = writeln!(
                out,
                "  {}",
                color::header(&format!("Suspended ({}):", suspended_jobs.len()))
            );
            let rows: Vec<JobRow> = suspended_jobs
                .iter()
                .map(|p| JobRow {
                    prefix: "    ".to_string(),
                    id: oj_core::short(&p.id, 8).to_string(),
                    name: friendly_name_label(&p.name, &p.kind, &p.id),
                    kind_step: format!("{}/{}", p.kind, p.step),
                    status: "suspended".to_string(),
                    suffix: oj_core::format_elapsed_ms(p.elapsed_ms),
                    reason: None,
                })
                .collect();
            write_aligned_job_rows(&mut out, &rows);
            out.push('\n');
        }

        // Workers
        if !workers.is_empty() {
            let _ = writeln!(out, "  {}", color::header("Workers:"));
            let w_name = workers.iter().map(|w| w.name.len()).max().unwrap_or(0);
            let labels: Vec<&str> = workers
                .iter()
                .map(|w| {
                    if w.status == "running" {
                        if w.active >= w.concurrency as usize {
                            "full"
                        } else {
                            "on"
                        }
                    } else {
                        "off"
                    }
                })
                .collect();
            let w_st = labels.iter().map(|l| l.len()).max().unwrap_or(0);
            for (w, label) in workers.iter().zip(labels.iter()) {
                let _ = writeln!(
                    out,
                    "    {:<w_name$}  {}  {}/{} active",
                    w.name,
                    color::status(&format!("{:<w_st$}", label)),
                    w.active,
                    w.concurrency,
                );
            }
            out.push('\n');
        }

        // Crons
        if !ns.crons.is_empty() {
            let mut crons: Vec<&oj_wire::CronSummary> = ns.crons.iter().collect();
            crons.sort_by(|a, b| a.name.cmp(&b.name));

            let _ = writeln!(out, "  {}", color::header("Crons:"));
            let c_name = crons.iter().map(|c| c.name.len()).max().unwrap_or(0);
            let c_int = crons.iter().map(|c| c.interval.len()).max().unwrap_or(0);
            let labels: Vec<&str> =
                crons.iter().map(|c| if c.status == "running" { "on" } else { "off" }).collect();
            let c_st = labels.iter().map(|l: &&str| l.len()).max().unwrap_or(0);
            for (c, label) in crons.iter().zip(labels.iter()) {
                let _ = writeln!(
                    out,
                    "    {:<c_name$}  {:<c_int$}  {}  {}",
                    c.name,
                    c.interval,
                    color::status(&format!("{:<c_st$}", label)),
                    c.time,
                );
            }
            out.push('\n');
        }

        // Queues
        let non_empty_queues: Vec<_> =
            ns.queues.iter().filter(|q| q.pending > 0 || q.active > 0 || q.dead > 0).collect();
        if !non_empty_queues.is_empty() {
            let _ = writeln!(out, "  {}", color::header("Queues:"));
            let w_name = non_empty_queues.iter().map(|q| q.name.len()).max().unwrap_or(0);
            for q in &non_empty_queues {
                let _ = write!(
                    out,
                    "    {:<w_name$}  {} pending, {} active",
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
            let w_name = ns.active_agents.iter().map(|a| a.agent_name.len()).max().unwrap_or(0);
            let w_st = ns.active_agents.iter().map(|a| a.status.len()).max().unwrap_or(0);
            for a in &ns.active_agents {
                let _ = writeln!(
                    out,
                    "    {}  {:<w_name$}  {}",
                    color::muted(oj_core::short(&a.agent_id, 8)),
                    a.agent_name,
                    color::status(&format!("{:<w_st$}", a.status)),
                );
            }
            out.push('\n');
        }
    }

    out
}

/// Returns the job name when it is a meaningful friendly name,
/// or an empty string when it would be redundant (same as kind) or opaque (same as id).
fn friendly_name_label(name: &str, kind: &str, id: &str) -> String {
    // Hide name when it's empty, matches the kind, or matches the full/truncated ID.
    // When the name template produces an empty slug, job_display_name() returns
    // just the nonce (first 8 chars of the ID), which would be redundant with the
    // truncated ID shown in the status output.
    let truncated_id = oj_core::short(id, 8);
    if name.is_empty() || name == kind || name == id || name == truncated_id {
        String::new()
    } else {
        name.to_string()
    }
}

/// A row of job data for aligned rendering.
struct JobRow {
    prefix: String,
    id: String,
    name: String,
    kind_step: String,
    status: String,
    suffix: String,
    reason: Option<String>,
}

/// Render job rows with aligned columns.
///
/// Columns: `{prefix}{id}  [{name}  ]{kind/step}  {status}  {suffix}`
/// The name column is omitted entirely when all names are empty.
fn write_aligned_job_rows(out: &mut String, rows: &[JobRow]) {
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
        let _ = write!(out, "  {}", color::context(&format!("{:<w_ks$}", r.kind_step)));
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
        let truncated = if first_line.len() > limit { &first_line[..limit] } else { first_line };
        format!("{}...", truncated)
    }
}

fn sorted_by_activity(jobs: &[oj_wire::JobStatusEntry]) -> Vec<&oj_wire::JobStatusEntry> {
    let mut sorted: Vec<_> = jobs.iter().collect();
    sorted.sort_by(|a, b| b.last_activity_ms.cmp(&a.last_activity_ms));
    sorted
}

#[cfg(test)]
#[path = "status_tests/mod.rs"]
mod tests;
