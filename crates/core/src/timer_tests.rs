// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::crew::CrewId;
use crate::job::JobId;

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
fn factory_methods_format() {
    assert_eq!(TimerId::liveness(&JobId::new("job-123")).as_str(), "liveness:job:job-123");
    assert_eq!(
        TimerId::exit_deferred(&JobId::new("job-123")).as_str(),
        "exit-deferred:job:job-123"
    );
    assert_eq!(
        TimerId::cooldown(&JobId::new("job-123"), "idle", 0).as_str(),
        "cooldown:job:job-123:idle:0"
    );
    assert_eq!(
        TimerId::cooldown(&JobId::new("job-456"), "exit", 2).as_str(),
        "cooldown:job:job-456:exit:2"
    );
    assert_eq!(TimerId::liveness(&CrewId::new("run-123")).as_str(), "liveness:agent:run-123");
    assert_eq!(
        TimerId::exit_deferred(&CrewId::new("run-123")).as_str(),
        "exit-deferred:agent:run-123"
    );
    assert_eq!(
        TimerId::cooldown(&CrewId::new("run-123"), "idle", 0).as_str(),
        "cooldown:agent:run-123:idle:0"
    );
    assert_eq!(TimerId::queue_retry("bugs", "item-123").as_str(), "queue-retry:bugs:item-123");
    assert_eq!(
        TimerId::queue_retry("myns/bugs", "item-456").as_str(),
        "queue-retry:myns/bugs:item-456"
    );
    assert_eq!(TimerId::cron("janitor", "").as_str(), "cron:janitor");
    assert_eq!(TimerId::cron("janitor", "myproject").as_str(), "cron:myproject/janitor");
    assert_eq!(TimerId::queue_poll("my-worker", "").as_str(), "queue-poll:my-worker");
    assert_eq!(
        TimerId::queue_poll("my-worker", "myproject").as_str(),
        "queue-poll:myproject/my-worker"
    );
}

#[test]
fn owner_id_constructors() {
    let job: OwnerId = JobId::new("job-123").into();
    let run: OwnerId = CrewId::new("run-456").into();
    assert_eq!(TimerId::liveness(&job).as_str(), "liveness:job:job-123");
    assert_eq!(TimerId::liveness(&run).as_str(), "liveness:agent:run-456");
    assert_eq!(TimerId::exit_deferred(&job).as_str(), "exit-deferred:job:job-123");
    assert_eq!(TimerId::exit_deferred(&run).as_str(), "exit-deferred:agent:run-456");
    assert_eq!(TimerId::cooldown(&job, "idle", 1).as_str(), "cooldown:job:job-123:idle:1");
    assert_eq!(TimerId::cooldown(&run, "exit", 2).as_str(), "cooldown:agent:run-456:exit:2");
}

#[test]
fn owner_id_extraction() {
    assert_eq!(
        TimerId::liveness(&JobId::new("job-123")).owner_id(),
        Some(OwnerId::Job(JobId::new("job-123")))
    );
    assert_eq!(
        TimerId::liveness(&CrewId::new("run-456")).owner_id(),
        Some(OwnerId::Crew(CrewId::new("run-456")))
    );
    assert_eq!(TimerId::cron("janitor", "").owner_id(), None);
}

#[test]
fn kind_unknown_returns_none() {
    assert!(TimerId::new("other-timer").kind().is_none());
}

#[test]
fn timer_kind_parse_unknown_returns_none() {
    assert!(TimerKind::parse("other-timer").is_none());
    assert!(TimerKind::parse("").is_none());
    assert!(TimerKind::parse("unknown:foo").is_none());
}

#[test]
fn timer_kind_round_trip_all_factory_methods() {
    let cases = vec![
        TimerId::liveness(&JobId::new("j1")),
        TimerId::exit_deferred(&JobId::new("j1")),
        TimerId::cooldown(&JobId::new("j1"), "idle", 0),
        TimerId::cooldown(&JobId::new("j1"), "exit", 3),
        TimerId::liveness(&CrewId::new("ar1")),
        TimerId::exit_deferred(&CrewId::new("ar1")),
        TimerId::cooldown(&CrewId::new("ar1"), "idle", 0),
        TimerId::cooldown(&CrewId::new("ar1"), "exit", 5),
        TimerId::queue_retry("bugs", "item-1"),
        TimerId::queue_retry("ns/bugs", "item-2"),
        TimerId::cron("janitor", ""),
        TimerId::cron("janitor", "myns"),
        TimerId::queue_poll("worker", ""),
        TimerId::queue_poll("worker", "myns"),
    ];

    for timer_id in &cases {
        let kind = TimerKind::parse(timer_id.as_str())
            .unwrap_or_else(|| panic!("failed to parse: {}", timer_id));
        let round_tripped = kind.to_timer_id();
        assert_eq!(timer_id, &round_tripped, "round-trip failed for: {}", timer_id);
    }
}
