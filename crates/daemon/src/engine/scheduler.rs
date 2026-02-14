// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Timer and scheduling management

use oj_core::{Event, TimerId};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Timer entry
#[derive(Debug, Clone)]
struct Timer {
    fires_at: Instant,
}

/// Manages timers for the runtime
#[derive(Debug, Default)]
pub struct Scheduler {
    timers: HashMap<String, Timer>,
}

impl Scheduler {
    /// Create a new scheduler
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a timer
    pub fn set_timer(&mut self, id: String, duration: Duration, now: Instant) {
        let fires_at = now + duration;
        self.timers.insert(id, Timer { fires_at });
    }

    /// Cancel a timer
    pub fn cancel_timer(&mut self, id: &str) {
        self.timers.remove(id);
    }

    /// Cancel all timers matching a prefix
    pub fn cancel_timers_with_prefix(&mut self, prefix: &str) {
        self.timers.retain(|id, _| !id.starts_with(prefix));
    }

    /// Get all timers that have fired
    pub fn fired_timers(&mut self, now: Instant) -> Vec<Event> {
        let mut events = Vec::new();
        let mut to_remove = Vec::new();

        for (id, timer) in &self.timers {
            if timer.fires_at <= now {
                events.push(Event::TimerStart { id: TimerId::from_string(id) });
                to_remove.push(id.clone());
            }
        }

        for id in to_remove {
            self.timers.remove(&id);
        }

        events
    }
}

#[cfg(test)]
impl Scheduler {
    pub fn next_deadline(&self) -> Option<Instant> {
        self.timers.values().map(|t| t.fires_at).min()
    }

    pub fn has_timers(&self) -> bool {
        !self.timers.is_empty()
    }
}

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;
