// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use clap::ValueEnum;
use serde::Serialize;

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;

/// Determine if color output should be enabled.
///
/// Delegates to [`crate::color::should_colorize`] — the single source of truth
/// for color detection across the CLI.
pub fn should_use_color() -> bool {
    crate::color::should_colorize()
}

/// Print a peek frame with box-drawing characters around session output.
pub fn print_peek_frame(session_id: &str, output: &str) {
    println!(
        "╭────── {} ──────",
        crate::color::header(&format!("peek: {}", session_id))
    );
    print!("{}", output);
    println!("╰────── {} ──────", crate::color::header("end peek"));
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
            println!(
                "\n{} {}(s) {}, {} {}",
                pruned.len(),
                entity,
                verb,
                skipped,
                skipped_label
            );
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

/// Display log content with optional follow mode, handling text/json output.
pub async fn display_log(
    log_path: &std::path::Path,
    content: &str,
    follow: bool,
    format: OutputFormat,
    label: &str,
    id: &str,
) -> anyhow::Result<()> {
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
                    return Ok(());
                }
            }

            if follow {
                tail_file(log_path).await?;
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
    Ok(())
}

/// Tail a file, printing new lines as they appear.
pub async fn tail_file(path: &std::path::Path) -> anyhow::Result<()> {
    use notify::{Event, EventKind, RecursiveMode, Watcher};
    use std::io::{BufRead, BufReader, Seek, SeekFrom};

    let mut file = std::fs::File::open(path)
        .map_err(|_| anyhow::anyhow!("Log file not found: {}", path.display()))?;
    // Seek to end — we already printed the tail above
    file.seek(SeekFrom::End(0))?;
    let mut reader = BufReader::new(file);

    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let path_buf = path.to_path_buf();

    // Watch for file modifications
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
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
