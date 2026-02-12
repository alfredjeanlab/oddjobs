// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn system_clock_returns_increasing_time() {
    let clock = SystemClock;
    let t1 = clock.now();
    std::thread::sleep(Duration::from_millis(1));
    let t2 = clock.now();
    assert!(t2 > t1);
}

#[test]
fn system_clock_default() {
    let clock = SystemClock;
    let _ = clock.now();
}

#[test]
fn fake_clock_can_be_advanced() {
    let clock = FakeClock::new();
    let t1 = clock.now();
    clock.advance(Duration::from_secs(60));
    let t2 = clock.now();
    assert!(t2.duration_since(t1) >= Duration::from_secs(60));
}

#[test]
fn fake_clock_is_cloneable_and_shared() {
    let clock1 = FakeClock::new();
    let clock2 = clock1.clone();
    let t1 = clock1.now();
    clock2.advance(Duration::from_secs(30));
    let t2 = clock1.now();
    assert!(t2.duration_since(t1) >= Duration::from_secs(30));
}

#[test]
fn fake_clock_default() {
    let clock = FakeClock::default();
    let _ = clock.now();
}

#[test]
fn fake_clock_set() {
    let clock = FakeClock::new();
    let future = Instant::now() + Duration::from_secs(3600);
    clock.set(future);
    assert!(clock.now() >= future);
}
