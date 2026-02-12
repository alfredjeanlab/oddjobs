// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Timer identifier type for tracking scheduled timers.
//!
//! TimerId uniquely identifies a timer instance used for scheduling delayed
//! actions such as timeouts, heartbeats, or periodic checks.

use crate::crew::CrewId;
use crate::job::JobId;
use crate::owner::OwnerId;
use crate::project::scoped_name;

crate::define_id! {
    /// Unique identifier for a timer instance.
    ///
    /// Timers are used to schedule delayed actions within the system, such as
    /// step timeouts or periodic health checks.
    pub struct TimerId;
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
        Self::new(format!("cron:{}", scoped_name(project, cron_name)))
    }

    pub fn queue_poll(worker_name: &str, project: &str) -> Self {
        Self::new(format!("queue-poll:{}", scoped_name(project, worker_name)))
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
            TimerKind::Cron { scoped_name } => TimerId::new(format!("cron:{scoped_name}")),
            TimerKind::Liveness(o) => TimerId::new(format!("liveness:{}", owner_segment(o))),
            TimerKind::ExitDeferred(o) => {
                TimerId::new(format!("exit-deferred:{}", owner_segment(o)))
            }
            TimerKind::Cooldown { owner, trigger, chain_pos } => {
                TimerId::new(format!("cooldown:{}:{}:{}", owner_segment(owner), trigger, chain_pos))
            }
            TimerKind::QueueRetry { scoped_queue, item_id } => {
                TimerId::new(format!("queue-retry:{scoped_queue}:{item_id}"))
            }
            TimerKind::QueuePoll { scoped_name } => {
                TimerId::new(format!("queue-poll:{scoped_name}"))
            }
        }
    }
}

/// Parse an owner segment from a timer ID string.
///
/// Returns `(owner, remaining)` where remaining is text after the owner's id
/// (separated by `:`), or empty if the owner consumes the full string.
///
/// Format: `job:<id>` or `agent:<id>`.
fn parse_owner(s: &str) -> Option<(OwnerId, &str)> {
    let (prefix, rest) = if let Some(r) = s.strip_prefix("job:") {
        ("job", r)
    } else if let Some(r) = s.strip_prefix("agent:") {
        ("agent", r)
    } else {
        return None;
    };
    let (id, remaining) = rest.split_once(':').unwrap_or((rest, ""));
    let owner = match prefix {
        "agent" => CrewId::new(id).into(),
        _ => JobId::new(id).into(),
    };
    Some((owner, remaining))
}

fn owner_segment(owner: &OwnerId) -> String {
    match owner {
        OwnerId::Job(id) => format!("job:{id}"),
        OwnerId::Crew(id) => format!("agent:{id}"),
    }
}

#[cfg(test)]
#[path = "timer_tests.rs"]
mod tests;
