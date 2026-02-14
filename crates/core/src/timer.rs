// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Timer identifier type for tracking scheduled timers.
//!
//! TimerId uniquely identifies a timer instance used for scheduling delayed
//! actions such as timeouts, heartbeats, or periodic checks.

use crate::owner::OwnerId;
use crate::project::scoped_name;

crate::define_id! {
    /// Unique identifier for a timer instance.
    ///
    /// Timers are used to schedule delayed actions within the system, such as
    /// step timeouts or periodic health checks.
    pub struct TimerId("tmr-");
}

impl TimerId {
    pub fn liveness(owner: impl Into<OwnerId>) -> Self {
        TimerKind::Liveness(owner.into()).to_timer_id()
    }

    pub fn exit_deferred(owner: impl Into<OwnerId>) -> Self {
        TimerKind::ExitDeferred(owner.into()).to_timer_id()
    }

    pub fn cooldown(owner: impl Into<OwnerId>, trigger: &str, chain_pos: usize) -> Self {
        TimerKind::Cooldown { owner: owner.into(), trigger, chain_pos }.to_timer_id()
    }

    pub fn queue_retry(queue: &str, item_id: &str) -> Self {
        TimerKind::QueueRetry { scoped_queue: queue, item_id }.to_timer_id()
    }

    pub fn cron(cron_name: &str, project: &str) -> Self {
        Self::from_string(format!("cron:{}", scoped_name(project, cron_name)))
    }

    pub fn queue_poll(worker_name: &str, project: &str) -> Self {
        Self::from_string(format!("queue-poll:{}", scoped_name(project, worker_name)))
    }

    /// Parse this timer ID into a typed `TimerKind`.
    pub fn kind(&self) -> Option<TimerKind<'_>> {
        TimerKind::parse(self.as_str())
    }
    /// Extract the OwnerId if this timer is associated with an owner.
    pub fn owner_id(&self) -> Option<OwnerId> {
        match self.kind()? {
            TimerKind::Liveness(owner)
            | TimerKind::ExitDeferred(owner)
            | TimerKind::Cooldown { owner, .. } => Some(owner),
            _ => None,
        }
    }
}

/// Parsed representation of a timer ID for type-safe routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerKind<'a> {
    Liveness(OwnerId),
    ExitDeferred(OwnerId),
    Cooldown { owner: OwnerId, trigger: &'a str, chain_pos: usize },
    QueueRetry { scoped_queue: &'a str, item_id: &'a str },
    Cron { scoped_name: &'a str },
    QueuePoll { scoped_name: &'a str },
}

impl<'a> TimerKind<'a> {
    /// Parse a timer ID string into a typed `TimerKind`.
    ///
    /// Returns `None` for unrecognized timer ID formats.
    pub fn parse(id: &'a str) -> Option<TimerKind<'a>> {
        if let Some(rest) = id.strip_prefix("liveness:") {
            return Some(TimerKind::Liveness(parse_owner(rest)?.0));
        }
        if let Some(rest) = id.strip_prefix("exit-deferred:") {
            return Some(TimerKind::ExitDeferred(parse_owner(rest)?.0));
        }
        if let Some(rest) = id.strip_prefix("cooldown:") {
            let (owner, after) = parse_owner(rest)?;
            if after.is_empty() {
                return None;
            }
            let (trigger, chain_pos_str) = after.rsplit_once(':')?;
            return Some(TimerKind::Cooldown {
                owner,
                trigger,
                chain_pos: chain_pos_str.parse().unwrap_or(0),
            });
        }
        if let Some(rest) = id.strip_prefix("queue-retry:") {
            let (scoped_queue, item_id) = rest.rsplit_once(':')?;
            return Some(TimerKind::QueueRetry { scoped_queue, item_id });
        }
        if let Some(rest) = id.strip_prefix("cron:") {
            return Some(TimerKind::Cron { scoped_name: rest });
        }
        if let Some(rest) = id.strip_prefix("queue-poll:") {
            return Some(TimerKind::QueuePoll { scoped_name: rest });
        }
        None
    }

    /// Format this `TimerKind` back into a canonical `TimerId`.
    pub fn to_timer_id(&self) -> TimerId {
        match self {
            TimerKind::Cron { scoped_name } => TimerId::from_string(format!("cron:{scoped_name}")),
            TimerKind::Liveness(o) => TimerId::from_string(format!("liveness:{o}")),
            TimerKind::ExitDeferred(o) => TimerId::from_string(format!("exit-deferred:{o}")),
            TimerKind::Cooldown { owner, trigger, chain_pos } => {
                TimerId::from_string(format!("cooldown:{owner}:{trigger}:{chain_pos}"))
            }
            TimerKind::QueueRetry { scoped_queue, item_id } => {
                TimerId::from_string(format!("queue-retry:{scoped_queue}:{item_id}"))
            }
            TimerKind::QueuePoll { scoped_name } => {
                TimerId::from_string(format!("queue-poll:{scoped_name}"))
            }
        }
    }
}

/// Parse an owner segment from a timer ID string.
///
/// Returns `(owner, remaining)` where remaining is text after the owner's id
/// (separated by `:`), or empty if the owner consumes the full string.
///
/// Format: `job-<id>` or `crw-<id>`.
fn parse_owner(s: &str) -> Option<(OwnerId, &str)> {
    // Split on the first ':' to get the owner ID and remaining
    let (owner_str, remaining) = s.split_once(':').unwrap_or((s, ""));

    // Parse the owner ID
    let owner = if owner_str.starts_with("job-") || owner_str.starts_with("crw-") {
        OwnerId::parse(owner_str).ok()?
    } else {
        return None;
    };

    Some((owner, remaining))
}

#[cfg(test)]
#[path = "timer_tests.rs"]
mod tests;
