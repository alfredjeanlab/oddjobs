// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::{Clock, FakeClock};

#[test]
fn scheduler_timer_lifecycle() {
    let clock = FakeClock::new();
    let mut scheduler = Scheduler::new();

    scheduler.set_timer("test".to_string(), Duration::from_secs(10), clock.now());
    assert!(scheduler.has_timers());
    assert!(scheduler.next_deadline().is_some());

    // Timer hasn't fired yet
    clock.advance(Duration::from_secs(5));
    let events = scheduler.fired_timers(clock.now());
    assert!(events.is_empty());
    assert!(scheduler.has_timers());

    // Timer fires
    clock.advance(Duration::from_secs(10));
    let events = scheduler.fired_timers(clock.now());
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::TimerStart { ref id } if id == "test"));
    assert!(!scheduler.has_timers());
}

#[test]
fn scheduler_cancel_timer() {
    let clock = FakeClock::new();
    let mut scheduler = Scheduler::new();

    scheduler.set_timer("test".to_string(), Duration::from_secs(10), clock.now());
    scheduler.cancel_timer("test");

    clock.advance(Duration::from_secs(15));
    let events = scheduler.fired_timers(clock.now());
    assert!(events.is_empty());
}

#[test]
fn scheduler_multiple_timers_fire_independently() {
    let clock = FakeClock::new();
    let mut scheduler = Scheduler::new();

    scheduler.set_timer("fast".to_string(), Duration::from_secs(5), clock.now());
    scheduler.set_timer("slow".to_string(), Duration::from_secs(20), clock.now());

    // Only fast timer fires at 6s
    clock.advance(Duration::from_secs(6));
    let events = scheduler.fired_timers(clock.now());
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::TimerStart { ref id } if id == "fast"));
    assert!(scheduler.has_timers(), "slow timer should still be pending");

    // Slow timer fires at 21s
    clock.advance(Duration::from_secs(15));
    let events = scheduler.fired_timers(clock.now());
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::TimerStart { ref id } if id == "slow"));
    assert!(!scheduler.has_timers());
}

#[test]
fn scheduler_next_deadline_returns_earliest() {
    let clock = FakeClock::new();
    let mut scheduler = Scheduler::new();

    scheduler.set_timer("later".to_string(), Duration::from_secs(30), clock.now());
    scheduler.set_timer("sooner".to_string(), Duration::from_secs(10), clock.now());

    let deadline = scheduler.next_deadline().unwrap();
    // The earliest deadline should be ~10s from start
    let expected = clock.now() + Duration::from_secs(10);
    assert_eq!(deadline, expected);
}

#[test]
fn scheduler_overwrite_timer_resets_deadline() {
    let clock = FakeClock::new();
    let mut scheduler = Scheduler::new();

    scheduler.set_timer("t".to_string(), Duration::from_secs(10), clock.now());

    // Overwrite with a longer duration
    clock.advance(Duration::from_secs(2));
    scheduler.set_timer("t".to_string(), Duration::from_secs(20), clock.now());

    // Original deadline (10s) should not fire
    clock.advance(Duration::from_secs(9));
    let events = scheduler.fired_timers(clock.now());
    assert!(events.is_empty(), "old timer deadline should be overwritten");

    // New deadline (20s from overwrite) should fire
    clock.advance(Duration::from_secs(12));
    let events = scheduler.fired_timers(clock.now());
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::TimerStart { ref id } if id == "t"));
}

#[test]
fn scheduler_empty_has_no_deadline() {
    let scheduler = Scheduler::new();
    assert!(!scheduler.has_timers());
    assert!(scheduler.next_deadline().is_none());
}

#[test]
fn scheduler_fired_timers_removes_only_expired() {
    let clock = FakeClock::new();
    let mut scheduler = Scheduler::new();

    scheduler.set_timer("a".to_string(), Duration::from_secs(5), clock.now());
    scheduler.set_timer("b".to_string(), Duration::from_secs(10), clock.now());
    scheduler.set_timer("c".to_string(), Duration::from_secs(15), clock.now());

    // Advance to 11s — a and b should fire, c should remain
    clock.advance(Duration::from_secs(11));
    let events = scheduler.fired_timers(clock.now());
    assert_eq!(events.len(), 2);

    let ids: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(ids.contains(&"a"));
    assert!(ids.contains(&"b"));

    assert!(scheduler.has_timers(), "timer c should still be pending");

    // c fires at 16s
    clock.advance(Duration::from_secs(5));
    let events = scheduler.fired_timers(clock.now());
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], Event::TimerStart { ref id } if id == "c"));
}

#[test]
fn scheduler_cancel_timers_with_prefix() {
    let clock = FakeClock::new();
    let mut scheduler = Scheduler::new();

    // Set up timers with different prefixes
    scheduler.set_timer("job:abc123".to_string(), Duration::from_secs(10), clock.now());
    scheduler.set_timer("job:abc123:step1".to_string(), Duration::from_secs(15), clock.now());
    scheduler.set_timer("job:def456".to_string(), Duration::from_secs(20), clock.now());
    scheduler.set_timer("other:xyz".to_string(), Duration::from_secs(25), clock.now());

    // Cancel all timers starting with "job:abc123"
    scheduler.cancel_timers_with_prefix("job:abc123");

    // Advance past all deadlines
    clock.advance(Duration::from_secs(30));
    let events = scheduler.fired_timers(clock.now());

    // Only "job:def456" and "other:xyz" should fire
    assert_eq!(events.len(), 2);
    let ids: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            Event::TimerStart { id } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert!(ids.contains(&"job:def456"));
    assert!(ids.contains(&"other:xyz"));
    assert!(!ids.contains(&"job:abc123"));
    assert!(!ids.contains(&"job:abc123:step1"));
}
