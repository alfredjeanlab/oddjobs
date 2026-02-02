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
