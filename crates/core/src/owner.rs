// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Owner identification for agent events.
//!
//! Agents can be owned by either a Job (job-embedded agents) or an Crew
//! (standalone agents). This module provides a tagged union type to represent
//! that ownership, enabling proper routing during WAL replay.

use crate::crew::CrewId;
use crate::job::JobId;
use std::fmt;

/// Owner of an agent event.
///
/// Used to route agent state events (Working, Waiting, Failed, Exited, Gone)
/// to the correct entity during WAL replay.
///
/// Serializes as a string using Display format:
/// - `"job-V1StGXR8_Zm5M9z3x"`
/// - `"crw-kL9mP2nQ_Az8Nk4wx"`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OwnerId {
    /// Agent is owned by a job (job-embedded agent)
    Job(JobId),
    /// Agent is owned by an crew (standalone agent)
    Crew(CrewId),
}

impl serde::Serialize for OwnerId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for OwnerId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        OwnerId::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl OwnerId {
    /// Create a Job owner.
    pub fn job(id: JobId) -> Self {
        OwnerId::Job(id)
    }

    /// Create a Crew owner.
    pub fn crew(id: CrewId) -> Self {
        OwnerId::Crew(id)
    }

    /// Returns the job ID if this is a Job owner.
    pub fn as_job(&self) -> Option<&JobId> {
        match self {
            OwnerId::Job(id) => Some(id),
            OwnerId::Crew(_) => None,
        }
    }

    /// Returns the crew ID if this is a Crew owner.
    pub fn as_crew(&self) -> Option<&CrewId> {
        match self {
            OwnerId::Crew(id) => Some(id),
            OwnerId::Job(_) => None,
        }
    }

    /// Returns the job ID or an error if this is not a Job owner.
    pub fn try_job(&self) -> Result<&JobId, OwnerMismatch> {
        match self {
            OwnerId::Job(id) => Ok(id),
            _ => Err(OwnerMismatch("job")),
        }
    }

    /// Returns the crew ID or an error if this is not a Crew owner.
    pub fn try_crew(&self) -> Result<&CrewId, OwnerMismatch> {
        match self {
            OwnerId::Crew(id) => Ok(id),
            _ => Err(OwnerMismatch("crew")),
        }
    }

    /// Parse from Display format (`"job-xxx"` / `"crw-xxx"`).
    pub fn parse(s: &str) -> Result<Self, InvalidOwnerId> {
        if s.starts_with("job-") {
            Ok(OwnerId::Job(JobId::from_string(s)))
        } else if s.starts_with("crw-") {
            Ok(OwnerId::Crew(CrewId::from_string(s)))
        } else {
            Err(InvalidOwnerId(s.to_string()))
        }
    }

    pub fn log(&self) -> String {
        match self {
            OwnerId::Job(id) => format!("job={}", id),
            OwnerId::Crew(id) => format!("crew={}", id),
        }
    }
}

/// Invalid owner ID format (expected `job-xxx` or `crw-xxx`).
#[derive(Debug, Clone)]
pub struct InvalidOwnerId(pub String);

impl fmt::Display for InvalidOwnerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid owner id format: {}", self.0)
    }
}

impl std::error::Error for InvalidOwnerId {}

/// Expected a specific [`OwnerId`] variant.
#[derive(Debug, Clone)]
pub struct OwnerMismatch(&'static str);

impl fmt::Display for OwnerMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expected {} owner", self.0)
    }
}

impl std::error::Error for OwnerMismatch {}

impl fmt::Display for OwnerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OwnerId::Job(id) => write!(f, "{}", id),
            OwnerId::Crew(id) => write!(f, "{}", id),
        }
    }
}

impl From<JobId> for OwnerId {
    fn from(id: JobId) -> Self {
        OwnerId::Job(id)
    }
}

impl From<&JobId> for OwnerId {
    fn from(id: &JobId) -> Self {
        OwnerId::Job(id.clone())
    }
}

impl From<CrewId> for OwnerId {
    fn from(id: CrewId) -> Self {
        OwnerId::Crew(id)
    }
}

impl From<&CrewId> for OwnerId {
    fn from(id: &CrewId) -> Self {
        OwnerId::Crew(id.clone())
    }
}

impl From<&OwnerId> for OwnerId {
    fn from(id: &OwnerId) -> Self {
        id.clone()
    }
}

impl PartialEq<CrewId> for OwnerId {
    fn eq(&self, other: &CrewId) -> bool {
        matches!(self, OwnerId::Crew(id) if id == other)
    }
}

impl PartialEq<OwnerId> for CrewId {
    fn eq(&self, other: &OwnerId) -> bool {
        other == self
    }
}

impl PartialEq<JobId> for OwnerId {
    fn eq(&self, other: &JobId) -> bool {
        matches!(self, OwnerId::Job(id) if id == other)
    }
}

impl PartialEq<OwnerId> for JobId {
    fn eq(&self, other: &OwnerId) -> bool {
        other == self
    }
}

#[cfg(test)]
#[path = "owner_test.rs"]
mod tests;
