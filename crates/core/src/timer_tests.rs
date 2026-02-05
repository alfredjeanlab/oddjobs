// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::agent_run::AgentRunId;
use crate::job::JobId;
use crate::owner::OwnerId;

#[test]
fn timer_id_display() {
    let id = TimerId::new("test-timer");
    assert_eq!(id.to_string(), "test-timer");
}

#[test]
fn timer_id_equality() {
    let id1 = TimerId::new("timer-1");
    let id2 = TimerId::new("timer-1");
    let id3 = TimerId::new("timer-2");

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
}

#[test]
fn timer_id_from_str() {
    let id: TimerId = "test".into();
    assert_eq!(id.as_str(), "test");
}

#[test]
fn timer_id_serde() {
    let id = TimerId::new("my-timer");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"my-timer\"");

    let parsed: TimerId = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, id);
}

#[test]
fn liveness_timer_id() {
    let job_id = JobId::new("pipe-123");
    let id = TimerId::liveness(&job_id);
    assert_eq!(id.as_str(), "liveness:pipe-123");
}

#[test]
fn exit_deferred_timer_id() {
    let job_id = JobId::new("pipe-123");
    let id = TimerId::exit_deferred(&job_id);
    assert_eq!(id.as_str(), "exit-deferred:pipe-123");
}

#[test]
fn cooldown_timer_id_format() {
    let job_id = JobId::new("pipe-123");
    let id = TimerId::cooldown(&job_id, "idle", 0);
    assert_eq!(id.as_str(), "cooldown:pipe-123:idle:0");

    let job_id2 = JobId::new("pipe-456");
    let id2 = TimerId::cooldown(&job_id2, "exit", 2);
    assert_eq!(id2.as_str(), "cooldown:pipe-456:exit:2");
}

#[test]
fn is_liveness() {
    let id = TimerId::new("liveness:pipe-123");
    assert!(id.is_liveness());

    let id = TimerId::new("exit-deferred:pipe-123");
    assert!(!id.is_liveness());

    let id = TimerId::new("cooldown:pipe-123:idle:0");
    assert!(!id.is_liveness());
}

#[test]
fn is_exit_deferred() {
    let id = TimerId::new("exit-deferred:pipe-123");
    assert!(id.is_exit_deferred());

    let id = TimerId::new("liveness:pipe-123");
    assert!(!id.is_exit_deferred());

    let id = TimerId::new("cooldown:pipe-123:idle:0");
    assert!(!id.is_exit_deferred());
}

#[test]
fn is_cooldown() {
    let id = TimerId::new("cooldown:pipe-123:idle:0");
    assert!(id.is_cooldown());

    let id = TimerId::new("liveness:pipe-123");
    assert!(!id.is_cooldown());

    let id = TimerId::new("exit-deferred:pipe-123");
    assert!(!id.is_cooldown());
}

#[test]
fn job_id_str_liveness() {
    let id = TimerId::new("liveness:pipe-123");
    assert_eq!(id.job_id_str(), Some("pipe-123"));
}

#[test]
fn job_id_str_exit_deferred() {
    let id = TimerId::new("exit-deferred:pipe-456");
    assert_eq!(id.job_id_str(), Some("pipe-456"));
}

#[test]
fn job_id_str_cooldown() {
    let id = TimerId::new("cooldown:pipe-789:idle:0");
    assert_eq!(id.job_id_str(), Some("pipe-789"));
}

#[test]
fn job_id_str_unknown_timer() {
    let id = TimerId::new("other-timer");
    assert_eq!(id.job_id_str(), None);
}

#[test]
fn queue_retry_timer_id_format() {
    let id = TimerId::queue_retry("bugs", "item-123");
    assert_eq!(id.as_str(), "queue-retry:bugs:item-123");
}

#[test]
fn queue_retry_timer_id_with_namespace() {
    let id = TimerId::queue_retry("myns/bugs", "item-456");
    assert_eq!(id.as_str(), "queue-retry:myns/bugs:item-456");
}

#[test]
fn is_queue_retry() {
    let id = TimerId::queue_retry("bugs", "item-1");
    assert!(id.is_queue_retry());

    let id = TimerId::new("liveness:pipe-123");
    assert!(!id.is_queue_retry());

    let id = TimerId::new("cooldown:pipe-123:idle:0");
    assert!(!id.is_queue_retry());
}

#[test]
fn cron_timer_id_format() {
    let id = TimerId::cron("janitor", "");
    assert_eq!(id.as_str(), "cron:janitor");
}

#[test]
fn cron_timer_id_with_namespace() {
    let id = TimerId::cron("janitor", "myproject");
    assert_eq!(id.as_str(), "cron:myproject/janitor");
}

#[test]
fn is_cron() {
    let id = TimerId::cron("janitor", "");
    assert!(id.is_cron());

    let id = TimerId::cron("janitor", "myproject");
    assert!(id.is_cron());

    let id = TimerId::new("liveness:pipe-123");
    assert!(!id.is_cron());
}

#[test]
fn queue_poll_timer_id_format() {
    let id = TimerId::queue_poll("my-worker", "");
    assert_eq!(id.as_str(), "queue-poll:my-worker");
}

#[test]
fn queue_poll_timer_id_with_namespace() {
    let id = TimerId::queue_poll("my-worker", "myproject");
    assert_eq!(id.as_str(), "queue-poll:myproject/my-worker");
}

#[test]
fn is_queue_poll() {
    let id = TimerId::queue_poll("my-worker", "");
    assert!(id.is_queue_poll());

    let id = TimerId::queue_poll("my-worker", "ns");
    assert!(id.is_queue_poll());

    let id = TimerId::new("liveness:pipe-123");
    assert!(!id.is_queue_poll());

    let id = TimerId::new("cron:janitor");
    assert!(!id.is_queue_poll());
}

// =============================================================================
// OwnerId-based constructor tests
// =============================================================================

#[test]
fn owner_liveness_job() {
    let job_id = JobId::new("job-123");
    let owner = OwnerId::job(job_id.clone());
    let id = TimerId::owner_liveness(&owner);
    assert_eq!(id.as_str(), "liveness:job-123");
}

#[test]
fn owner_liveness_agent_run() {
    let ar_id = AgentRunId::new("ar-456");
    let owner = OwnerId::agent_run(ar_id.clone());
    let id = TimerId::owner_liveness(&owner);
    assert_eq!(id.as_str(), "liveness:ar:ar-456");
}

#[test]
fn owner_exit_deferred_job() {
    let job_id = JobId::new("job-123");
    let owner = OwnerId::job(job_id.clone());
    let id = TimerId::owner_exit_deferred(&owner);
    assert_eq!(id.as_str(), "exit-deferred:job-123");
}

#[test]
fn owner_exit_deferred_agent_run() {
    let ar_id = AgentRunId::new("ar-456");
    let owner = OwnerId::agent_run(ar_id.clone());
    let id = TimerId::owner_exit_deferred(&owner);
    assert_eq!(id.as_str(), "exit-deferred:ar:ar-456");
}

#[test]
fn owner_cooldown_job() {
    let job_id = JobId::new("job-123");
    let owner = OwnerId::job(job_id.clone());
    let id = TimerId::owner_cooldown(&owner, "idle", 1);
    assert_eq!(id.as_str(), "cooldown:job-123:idle:1");
}

#[test]
fn owner_cooldown_agent_run() {
    let ar_id = AgentRunId::new("ar-456");
    let owner = OwnerId::agent_run(ar_id.clone());
    let id = TimerId::owner_cooldown(&owner, "exit", 2);
    assert_eq!(id.as_str(), "cooldown:ar:ar-456:exit:2");
}

#[test]
fn owner_idle_grace_job() {
    let job_id = JobId::new("job-123");
    let owner = OwnerId::job(job_id.clone());
    let id = TimerId::owner_idle_grace(&owner);
    assert_eq!(id.as_str(), "idle-grace:job-123");
}

#[test]
fn owner_idle_grace_agent_run() {
    let ar_id = AgentRunId::new("ar-456");
    let owner = OwnerId::agent_run(ar_id.clone());
    let id = TimerId::owner_idle_grace(&owner);
    assert_eq!(id.as_str(), "idle-grace:ar:ar-456");
}

#[test]
fn owner_id_extracts_job() {
    let id = TimerId::liveness(&JobId::new("job-123"));
    let owner = id.owner_id();
    assert_eq!(owner, Some(OwnerId::job(JobId::new("job-123"))));
}

#[test]
fn owner_id_extracts_agent_run() {
    let id = TimerId::liveness_agent_run(&AgentRunId::new("ar-456"));
    let owner = id.owner_id();
    assert_eq!(owner, Some(OwnerId::agent_run(AgentRunId::new("ar-456"))));
}

#[test]
fn owner_id_returns_none_for_unrelated() {
    let id = TimerId::cron("janitor", "");
    assert_eq!(id.owner_id(), None);
}

// =============================================================================
// job_id_str exclusion tests
// =============================================================================

#[test]
fn job_id_str_excludes_agent_run_liveness() {
    // Agent run timer: should NOT return a job_id
    let id = TimerId::liveness_agent_run(&AgentRunId::new("ar-123"));
    assert_eq!(id.job_id_str(), None);
}

#[test]
fn job_id_str_excludes_agent_run_cooldown() {
    let id = TimerId::cooldown_agent_run(&AgentRunId::new("ar-123"), "idle", 0);
    assert_eq!(id.job_id_str(), None);
}

#[test]
fn job_id_str_excludes_agent_run_idle_grace() {
    let id = TimerId::idle_grace_agent_run(&AgentRunId::new("ar-123"));
    assert_eq!(id.job_id_str(), None);
}
