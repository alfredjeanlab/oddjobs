// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};

use crate::client::{StartResult, StopResult};
use clap::ValueEnum;
use notify::{Event as NotifyEvent, EventKind, RecursiveMode, Watcher};
use serde::Serialize;

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;

/// Print a saved terminal capture with distinct framing from live peek.
pub fn print_capture_frame(label: &str, output: &str) {
    println!("╭── {} ──", crate::color::header(&format!("last capture: {}", label)));
    print!("{}", output);
    println!("╰── {} ──", crate::color::header("end capture"));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

/// Format a timestamp as relative time (e.g., "5s", "2m", "1h", "3d")
pub fn format_time_ago(epoch_ms: u64) -> String {
    if epoch_ms == 0 {
        return "-".to_string();
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let elapsed_secs = now_ms.saturating_sub(epoch_ms) / 1000;
    oj_core::format_elapsed(elapsed_secs)
}

/// Print prune results in text or JSON format.
///
/// Handles the dry-run header, per-entry formatting, and summary line that is
/// shared across all `oj <entity> prune` commands.
///
/// - `entity` — singular name shown in the summary, e.g. `"job"`.
/// - `skipped_label` — suffix after the skipped count, e.g. `"skipped"` or
///   `"active workspace(s) skipped"`.
/// - `format_entry` — returns the text to print after "Pruned" / "Would prune"
///   for each entry.
pub fn print_prune_results<T: Serialize>(
    pruned: &[T],
    skipped: usize,
    dry_run: bool,
    format: OutputFormat,
    entity: &str,
    skipped_label: &str,
    format_entry: impl Fn(&T) -> String,
) -> anyhow::Result<()> {
    match format {
        OutputFormat::Text => {
            if dry_run {
                println!("Dry run — no changes made\n");
            }

            let label = if dry_run { "Would prune" } else { "Pruned" };
            for entry in pruned {
                println!("{} {}", label, format_entry(entry));
            }

            let verb = if dry_run { "would be pruned" } else { "pruned" };
            println!("\n{} {}(s) {}, {} {}", pruned.len(), entity, verb, skipped, skipped_label);
        }
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "dry_run": dry_run,
                "pruned": pruned,
                "skipped": skipped,
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
    }
    Ok(())
}

/// Print start results for worker/cron commands.
///
/// Handles single-start and bulk-start (`--all`) output that is shared across
/// `oj worker start` and `oj cron start`.
///
/// - `label` — capitalized entity name, e.g. `"Worker"` or `"Cron"`.
/// - `plural` — lowercase plural, e.g. `"workers"` or `"crons"`.
pub fn print_start_results(
    result: &crate::client::StartResult,
    label: &str,
    plural: &str,
    project: &str,
) {
    match result {
        StartResult::Single { name } => {
            println!(
                "{} '{}' started ({})",
                label,
                crate::color::header(name),
                crate::color::muted(project)
            );
        }
        StartResult::Multiple { started, skipped } => {
            for name in started {
                println!(
                    "{} '{}' started ({})",
                    label,
                    crate::color::header(name),
                    crate::color::muted(project)
                );
            }
            for (name, reason) in skipped {
                println!(
                    "{} '{}' skipped: {}",
                    label,
                    crate::color::header(name),
                    crate::color::muted(reason)
                );
            }
            if started.is_empty() && skipped.is_empty() {
                println!("No {} found in runbooks", plural);
            }
        }
    }
}

/// Print stop results for worker commands.
///
/// Handles single-stop and bulk-stop (`--all`) output.
///
/// - `label` — capitalized entity name, e.g. `"Worker"`.
/// - `plural` — lowercase plural, e.g. `"workers"`.
pub fn print_stop_results(
    result: &crate::client::StopResult,
    label: &str,
    plural: &str,
    project: &str,
) {
    match result {
        StopResult::Single { name } => {
            println!(
                "{} '{}' stopped ({})",
                label,
                crate::color::header(name),
                crate::color::muted(project)
            );
        }
        StopResult::Multiple { stopped, skipped } => {
            for name in stopped {
                println!(
                    "{} '{}' stopped ({})",
                    label,
                    crate::color::header(name),
                    crate::color::muted(project)
                );
            }
            for (name, reason) in skipped {
                println!(
                    "{} '{}' skipped: {}",
                    label,
                    crate::color::header(name),
                    crate::color::muted(reason)
                );
            }
            if stopped.is_empty() && skipped.is_empty() {
                println!("No running {} found", plural);
            }
        }
    }
}

/// Display log content with optional follow mode, handling text/json output.
///
/// Returns the byte offset for polling if follow mode is needed but the log
/// file is not locally accessible. Callers should use [`poll_log_follow`] with
/// a query-specific callback when this returns `Some(offset)`.
pub async fn display_log(
    log_path: &std::path::Path,
    content: &str,
    follow: bool,
    offset: u64,
    format: OutputFormat,
    label: &str,
    id: &str,
) -> anyhow::Result<Option<u64>> {
    match format {
        OutputFormat::Text => {
            if !content.is_empty() {
                print!("{}", content);
                if !content.ends_with('\n') {
                    println!();
                }
            } else {
                eprintln!("No log entries found for {} {}", label, id);
                if !follow {
                    return Ok(None);
                }
            }

            if follow {
                if log_path.exists() {
                    // Local file tailing (event-driven, fast)
                    tail_file(log_path).await?;
                } else {
                    // File not locally accessible — caller should poll
                    return Ok(Some(offset));
                }
            }
        }
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "log_path": log_path.to_string_lossy(),
                "lines": content.lines().collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
            if follow {
                eprintln!("warning: --follow is not supported with --output json");
            }
        }
    }
    Ok(None)
}

/// Poll daemon for log updates in a loop until Ctrl-C.
///
/// `poll_fn` takes a byte offset and returns `(new_content, new_offset)`.
/// Called by command handlers when `display_log` returns `Some(offset)`.
pub async fn poll_log_follow<F, Fut>(mut offset: u64, poll_fn: F) -> anyhow::Result<()>
where
    F: Fn(u64) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<(String, u64)>>,
{
    let poll_ms: u64 =
        std::env::var("OJ_LOG_POLL_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(poll_ms)) => {
                match poll_fn(offset).await {
                    Ok((content, new_offset)) => {
                        if !content.is_empty() {
                            print!("{}", content);
                            let _ = std::io::stdout().flush();
                        }
                        offset = new_offset;
                    }
                    Err(_) => {
                        // Connection lost — retry on next poll
                    }
                }
            }
            _ = &mut ctrl_c => break,
        }
    }
    Ok(())
}

/// Print results from a bulk cancel/suspend operation.
///
/// `action_past` — e.g. "Cancelled" or "Suspended".
/// Exits with code 1 if any IDs were not found.
pub fn print_batch_action_results(
    actioned: &[String],
    action_past: &str,
    already_terminal: &[String],
    not_found: &[String],
) {
    for id in actioned {
        println!("{} job {}", action_past, id);
    }
    for id in already_terminal {
        println!("Job {} was already terminal", id);
    }
    for id in not_found {
        eprintln!("Job not found: {}", id);
    }
    if !not_found.is_empty() {
        std::process::exit(1);
    }
}

/// Validate that a name is provided (or --all was passed).
pub fn require_name_or_all(
    name: Option<String>,
    all: bool,
    entity: &str,
) -> anyhow::Result<String> {
    if !all && name.is_none() {
        anyhow::bail!("{} name required (or use --all)", entity);
    }
    Ok(name.unwrap_or_default())
}

/// Filter items by project project.
pub fn filter_by_project<T>(
    items: &mut Vec<T>,
    project: Option<&str>,
    get_namespace: impl Fn(&T) -> &str,
) {
    if let Some(proj) = project {
        items.retain(|item| get_namespace(item) == proj);
    }
}

/// Info about items that were truncated by [`apply_limit`].
pub struct Truncation {
    pub remaining: usize,
}

/// Apply limit/no_limit to a vec, return truncation info if items were removed.
pub fn apply_limit<T>(items: &mut Vec<T>, limit: usize, no_limit: bool) -> Option<Truncation> {
    let total = items.len();
    let effective = if no_limit { total } else { limit };
    if total > effective {
        items.truncate(effective);
        Some(Truncation { remaining: total - effective })
    } else {
        None
    }
}

/// Render a list as text table or JSON. Handles empty check + format branch.
pub fn handle_list<T: Serialize>(
    format: OutputFormat,
    items: &[T],
    empty_msg: &str,
    render_text: impl FnOnce(&[T], &mut dyn Write),
) -> anyhow::Result<()> {
    handle_list_with_limit(format, items, empty_msg, None, render_text)
}

/// Like [`handle_list`] but prints a truncation message when items were limited.
pub fn handle_list_with_limit<T: Serialize>(
    format: OutputFormat,
    items: &[T],
    empty_msg: &str,
    truncation: Option<Truncation>,
    render_text: impl FnOnce(&[T], &mut dyn Write),
) -> anyhow::Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(items)?);
        }
        OutputFormat::Text => {
            if items.is_empty() {
                println!("{}", empty_msg);
            } else {
                render_text(items, &mut std::io::stdout());
            }
            if let Some(trunc) = truncation {
                if trunc.remaining > 0 {
                    println!(
                        "\n... {} more not shown. Use --no-limit or -n N to see more.",
                        trunc.remaining
                    );
                }
            }
        }
    }
    Ok(())
}

/// Format-branch helper for non-list commands (show, resume, etc.).
///
/// Renders as JSON when `format` is `Json`, otherwise calls `text_fn`.
pub fn format_or_json<T: Serialize>(
    format: OutputFormat,
    data: &T,
    text_fn: impl FnOnce(),
) -> anyhow::Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(data)?);
        }
        OutputFormat::Text => {
            text_fn();
        }
    }
    Ok(())
}

/// Tail a file, printing new lines as they appear.
pub async fn tail_file(path: &std::path::Path) -> anyhow::Result<()> {
    let mut file = std::fs::File::open(path)
        .map_err(|_| anyhow::anyhow!("Log file not found: {}", path.display()))?;
    // Seek to end — we already printed the tail above
    file.seek(SeekFrom::End(0))?;
    let mut reader = BufReader::new(file);

    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let path_buf = path.to_path_buf();

    // Watch for file modifications
    let mut watcher = notify::recommended_watcher(move |res: Result<NotifyEvent, _>| {
        if let Ok(event) = res {
            if matches!(event.kind, EventKind::Modify(_)) {
                let _ = tx.blocking_send(());
            }
        }
    })?;
    let watch_dir = path_buf.parent().unwrap_or(&path_buf);
    watcher.watch(watch_dir, RecursiveMode::NonRecursive)?;

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        // Read any new lines
        let mut line = String::new();
        while reader.read_line(&mut line)? > 0 {
            print!("{}", line);
            line.clear();
        }

        // Wait for file modification (or ctrl-c)
        tokio::select! {
            _ = rx.recv() => {}
            _ = &mut ctrl_c => break,
        }
    }

    Ok(())
}
