// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent wait/polling logic.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::client::DaemonClient;
use crate::exit_error::ExitError;

use super::super::job::parse_duration;

pub(super) async fn handle_wait(
    agent_id: &str,
    timeout: Option<&str>,
    client: &DaemonClient,
) -> Result<()> {
    let timeout_dur = timeout.map(parse_duration).transpose()?;
    let mut poller = crate::poll::Poller::new(crate::client::wait_poll_interval(), timeout_dur);
    let start = Instant::now();

    // Resolve agent to a job on first iteration; re-scan if not found yet
    let mut resolved_job_id: Option<String> = None;
    let mut resolved_agent_id: Option<String> = None;

    loop {
        // If we haven't found the agent yet, search for it
        if resolved_job_id.is_none() {
            if let Some((pid, aid)) = find_agent(client, agent_id).await? {
                resolved_job_id = Some(pid);
                resolved_agent_id = Some(aid);
            }
        }

        let job_id = match &resolved_job_id {
            Some(id) => id.clone(),
            None => {
                // On first poll with no match, give a grace period for the agent to appear
                if start.elapsed() > Duration::from_secs(10) {
                    return Err(ExitError::new(3, format!("Agent not found: {}", agent_id)).into());
                }
                match poller.tick().await {
                    crate::poll::Tick::Ready => continue,
                    crate::poll::Tick::Timeout => {
                        return Err(ExitError::new(
                            2,
                            format!("Timeout waiting for agent {}", agent_id),
                        )
                        .into());
                    }
                    crate::poll::Tick::Interrupted => {
                        return Err(ExitError::new(130, String::new()).into());
                    }
                }
            }
        };

        let full_agent_id = resolved_agent_id.as_deref().unwrap_or(agent_id);

        let detail = client.get_job(&job_id).await?;
        match detail {
            None => {
                return Err(ExitError::new(3, format!("Job {} disappeared", job_id)).into());
            }
            Some(p) => {
                // Find our specific agent in the job
                let agent = p.agents.iter().find(|a| a.agent_id == full_agent_id);

                match agent {
                    Some(agent) => {
                        // Check agent-level terminal/idle states
                        match agent.status.as_str() {
                            "completed" => {
                                println!("Agent {} completed", full_agent_id);
                                break;
                            }
                            "waiting" => {
                                println!("Agent {} waiting", full_agent_id);
                                break;
                            }
                            "failed" => {
                                let reason =
                                    agent.exit_reason.as_deref().unwrap_or("unknown error");
                                return Err(ExitError::new(
                                    1,
                                    format!("Agent {} failed: {}", full_agent_id, reason),
                                )
                                .into());
                            }
                            _ => {
                                // Check exit_reason for agent-level terminals
                                match agent.exit_reason.as_deref() {
                                    Some("gone") => {
                                        return Err(ExitError::new(
                                            1,
                                            format!("Agent {} session gone", full_agent_id,),
                                        )
                                        .into());
                                    }
                                    Some(reason) if reason.starts_with("failed") => {
                                        return Err(ExitError::new(
                                            1,
                                            format!("Agent {} {}", full_agent_id, reason,),
                                        )
                                        .into());
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    None => {
                        // Agent no longer in the job's agent list — it finished
                        // and the job moved on. Treat as completed.
                        println!("Agent {} completed (no longer active)", full_agent_id);
                        break;
                    }
                }

                // Also check job-level terminal states as a fallback
                if p.step == "failed" {
                    let msg = p.error.as_deref().unwrap_or("unknown error");
                    return Err(ExitError::new(1, format!("Job {} failed: {}", p.name, msg)).into());
                }
                if p.step == "cancelled" {
                    return Err(ExitError::new(4, format!("Job {} was cancelled", p.name)).into());
                }
            }
        }
        match poller.tick().await {
            crate::poll::Tick::Ready => {}
            crate::poll::Tick::Timeout => {
                return Err(
                    ExitError::new(2, format!("Timeout waiting for agent {}", agent_id)).into(),
                );
            }
            crate::poll::Tick::Interrupted => {
                return Err(ExitError::new(130, String::new()).into());
            }
        }
    }

    Ok(())
}

/// Find an agent by ID (or prefix) across all jobs.
/// Returns `(job_id, agent_id)` on match, or None.
async fn find_agent(
    client: &DaemonClient,
    agent_id: &str,
) -> Result<Option<(String, String)>, anyhow::Error> {
    let jobs = client.list_jobs().await?;
    for summary in &jobs {
        if let Some(detail) = client.get_job(&summary.id).await? {
            for agent in &detail.agents {
                if agent.agent_id == agent_id || agent.agent_id.starts_with(agent_id) {
                    return Ok(Some((summary.id.clone(), agent.agent_id.clone())));
                }
            }
        }
    }
    Ok(None)
}
