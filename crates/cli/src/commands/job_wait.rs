// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj job wait` - Block until job(s) reach a terminal state

use std::collections::HashMap;

use anyhow::Result;

use oj_core::ShortId;

use crate::client::DaemonClient;
use crate::exit_error::ExitError;
use crate::poll::{Poller, Tick};

enum JobOutcome {
    Done,
    Failed(String),
    Cancelled,
}

/// Tracks step progress for a single job during wait polling.
pub(crate) struct StepTracker {
    /// Number of steps we've already printed final transitions for.
    pub(crate) printed_count: usize,
    /// Whether we've printed a "started" line for the current (not-yet-final) step.
    pub(crate) printed_started: bool,
}

pub async fn handle(
    ids: Vec<String>,
    all: bool,
    timeout: Option<String>,
    client: &DaemonClient,
) -> Result<()> {
    let timeout_dur = timeout
        .map(|s| super::job::parse_duration(&s))
        .transpose()?;
    let mut poller = Poller::new(crate::client::wait_poll_interval(), timeout_dur);

    let mut finished: HashMap<String, JobOutcome> = HashMap::new();
    let mut canonical_ids: HashMap<String, String> = HashMap::new();
    let mut step_trackers: HashMap<String, StepTracker> = HashMap::new();
    let show_prefix = ids.len() > 1;

    loop {
        for input_id in &ids {
            if finished.contains_key(input_id) {
                continue;
            }
            let detail = client.get_job(input_id).await?;
            match detail {
                None => {
                    return Err(ExitError::new(3, format!("Job not found: {}", input_id)).into());
                }
                Some(p) => {
                    canonical_ids
                        .entry(input_id.clone())
                        .or_insert_with(|| p.id.clone());

                    let tracker = step_trackers
                        .entry(input_id.clone())
                        .or_insert(StepTracker {
                            printed_count: 0,
                            printed_started: false,
                        });
                    let mut stdout = std::io::stdout();
                    print_step_progress(&p, tracker, show_prefix, &mut stdout);

                    let outcome = match p.step.as_str() {
                        "done" => Some(JobOutcome::Done),
                        "failed" => Some(JobOutcome::Failed(
                            p.error.clone().unwrap_or_else(|| "unknown error".into()),
                        )),
                        "cancelled" => Some(JobOutcome::Cancelled),
                        _ => None,
                    };
                    if let Some(outcome) = outcome {
                        let short_id = canonical_ids[input_id].short(8);
                        match &outcome {
                            JobOutcome::Done => {
                                println!("Job {} ({}) completed", p.name, short_id);
                            }
                            JobOutcome::Failed(msg) => {
                                eprintln!("Job {} ({}) failed: {}", p.name, short_id, msg);
                            }
                            JobOutcome::Cancelled => {
                                eprintln!("Job {} ({}) was cancelled", p.name, short_id);
                            }
                        }
                        finished.insert(input_id.clone(), outcome);
                    }
                }
            }
        }

        if all {
            if finished.len() == ids.len() {
                break;
            }
        } else if !finished.is_empty() {
            break;
        }

        match poller.tick().await {
            Tick::Ready => {}
            Tick::Timeout => {
                return Err(ExitError::new(2, "Timeout waiting for job(s)".to_string()).into());
            }
            Tick::Interrupted => {
                return Err(ExitError::new(130, String::new()).into());
            }
        }
    }

    let any_failed = finished
        .values()
        .any(|o| matches!(o, JobOutcome::Failed(_)));
    let any_cancelled = finished
        .values()
        .any(|o| matches!(o, JobOutcome::Cancelled));
    if any_failed {
        return Err(ExitError::new(1, String::new()).into());
    }
    if any_cancelled {
        return Err(ExitError::new(4, String::new()).into());
    }

    Ok(())
}

/// Print step transitions that occurred since the last poll.
pub(crate) fn print_step_progress(
    detail: &oj_daemon::JobDetail,
    tracker: &mut StepTracker,
    show_job_prefix: bool,
    out: &mut impl std::io::Write,
) {
    let prefix = if show_job_prefix {
        format!("[{}] ", detail.name)
    } else {
        String::new()
    };

    for (i, step) in detail.steps.iter().enumerate() {
        if i < tracker.printed_count {
            continue;
        }

        let is_terminal = matches!(step.outcome.as_str(), "completed" | "failed");

        if is_terminal {
            // Print "started" for steps we haven't announced yet (skipped running state)
            if i == tracker.printed_count && !tracker.printed_started {
                // Step completed between polls without us seeing "running" — don't print started
                // for instant steps, just print the final outcome directly.
            }

            let elapsed = format_duration(step.started_at_ms, step.finished_at_ms);
            match step.outcome.as_str() {
                "completed" => {
                    let _ = writeln!(out, "{}{} completed ({})", prefix, step.name, elapsed);
                }
                "failed" => {
                    let suffix = match &step.detail {
                        Some(d) if !d.is_empty() => format!(" - {}", d),
                        _ => String::new(),
                    };
                    let _ = writeln!(
                        out,
                        "{}{} failed ({}){}",
                        prefix, step.name, elapsed, suffix
                    );
                }
                _ => unreachable!(),
            }
            tracker.printed_count = i + 1;
            tracker.printed_started = false;
        } else if step.outcome == "running" && !tracker.printed_started {
            let _ = writeln!(out, "{}{} started", prefix, step.name);
            tracker.printed_started = true;
        } else if step.outcome == "waiting" && !tracker.printed_started {
            let reason = step.detail.as_deref().unwrap_or("waiting");
            let _ = writeln!(out, "{}{} waiting ({})", prefix, step.name, reason);
            tracker.printed_started = true;
        }
    }
}

pub(crate) fn format_duration(started_ms: u64, finished_ms: Option<u64>) -> String {
    let end = finished_ms.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    });
    let elapsed_secs = (end.saturating_sub(started_ms)) / 1000;
    if elapsed_secs < 60 {
        format!("{}s", elapsed_secs)
    } else if elapsed_secs < 3600 {
        format!("{}m {}s", elapsed_secs / 60, elapsed_secs % 60)
    } else {
        format!("{}h {}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
    }
}
