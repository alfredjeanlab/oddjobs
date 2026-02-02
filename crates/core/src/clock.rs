// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Clock abstraction for testable time handling

use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A clock that provides the current time
pub trait Clock: Clone + Send + Sync {
    fn now(&self) -> Instant;
    fn epoch_ms(&self) -> u64;
}

/// Real system clock
#[derive(Clone, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn epoch_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

/// Fake clock for testing with controllable time
#[derive(Clone)]
pub struct FakeClock {
    current: Arc<Mutex<Instant>>,
    epoch_ms: Arc<Mutex<u64>>,
}

impl FakeClock {
    pub fn new() -> Self {
        Self {
            current: Arc::new(Mutex::new(Instant::now())),
            epoch_ms: Arc::new(Mutex::new(1_000_000)),
        }
    }

    /// Advance the clock by the given duration
    pub fn advance(&self, duration: Duration) {
        *self.current.lock() += duration;
        *self.epoch_ms.lock() += duration.as_millis() as u64;
    }

    /// Set the clock to a specific instant
    pub fn set(&self, instant: Instant) {
        *self.current.lock() = instant;
    }

    /// Set the epoch milliseconds value
    pub fn set_epoch_ms(&self, ms: u64) {
        *self.epoch_ms.lock() = ms;
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.current.lock()
    }

    fn epoch_ms(&self) -> u64 {
        *self.epoch_ms.lock()
    }
}

#[cfg(test)]
#[path = "clock_tests.rs"]
mod tests;
