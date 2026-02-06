// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Polling loop helper for CLI commands.
//!
//! Consolidates the common pattern across CLI commands that poll the daemon
//! for state changes with configurable intervals, deadlines, and Ctrl+C support.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

/// Result of waiting for the next poll tick.
pub enum Tick {
    /// Ready for the next poll iteration.
    Ready,
    /// The deadline was reached.
    Timeout,
    /// Ctrl+C was pressed.
    Interrupted,
}

/// A polling loop helper with interval timing, optional deadline, and Ctrl+C handling.
pub struct Poller {
    interval: Duration,
    deadline: Option<Instant>,
    ctrl_c: Pin<Box<dyn Future<Output = std::io::Result<()>>>>,
}

impl Poller {
    /// Create a new poller with the given interval and optional timeout.
    pub fn new(interval: Duration, timeout: Option<Duration>) -> Self {
        Self {
            interval,
            deadline: timeout.map(|t| Instant::now() + t),
            ctrl_c: Box::pin(tokio::signal::ctrl_c()),
        }
    }

    /// Wait for the next poll tick.
    ///
    /// Returns [`Tick::Ready`] after sleeping for the configured interval.
    /// Returns [`Tick::Timeout`] if the deadline has been reached (checked
    /// both before and after sleeping). Returns [`Tick::Interrupted`] if
    /// Ctrl+C was pressed during the sleep.
    pub async fn tick(&mut self) -> Tick {
        if let Some(deadline) = self.deadline {
            if Instant::now() >= deadline {
                return Tick::Timeout;
            }
        }

        tokio::select! {
            _ = &mut self.ctrl_c => Tick::Interrupted,
            _ = tokio::time::sleep(self.interval) => {
                if let Some(deadline) = self.deadline {
                    if Instant::now() >= deadline {
                        return Tick::Timeout;
                    }
                }
                Tick::Ready
            }
        }
    }
}

#[cfg(test)]
#[path = "poll_tests.rs"]
mod tests;
