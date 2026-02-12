// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// What a command, worker, cron (or one-shot dispatch) should execute.
///
/// Serializes to/from a tagged string: `"job:name"`, `"agent:name"`, `"shell:cmd"`.
/// Bare names (no prefix) are treated as `Job` for backward compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RunTarget {
    Job(String),
    Agent(String),
    Shell(String),
}

impl RunTarget {
    pub fn job(name: impl Into<String>) -> Self {
        RunTarget::Job(name.into())
    }

    pub fn agent(name: impl Into<String>) -> Self {
        RunTarget::Agent(name.into())
    }

    pub fn shell(cmd: impl Into<String>) -> Self {
        RunTarget::Shell(cmd.into())
    }

    /// The inner name or command.
    pub fn name(&self) -> &str {
        match self {
            RunTarget::Job(n) | RunTarget::Agent(n) | RunTarget::Shell(n) => n,
        }
    }

    pub fn is_job(&self) -> bool {
        matches!(self, RunTarget::Job(_))
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, RunTarget::Agent(_))
    }

    pub fn is_shell(&self) -> bool {
        matches!(self, RunTarget::Shell(_))
    }

    pub fn log(&self) -> String {
        match self {
            RunTarget::Job(name) => format!("job={}", name),
            RunTarget::Agent(name) => format!("agent={}", name),
            RunTarget::Shell(cmd) => format!("shell={}", cmd),
        }
    }
}

impl fmt::Display for RunTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunTarget::Job(name) => write!(f, "job:{}", name),
            RunTarget::Agent(name) => write!(f, "agent:{}", name),
            RunTarget::Shell(cmd) => write!(f, "shell:{}", cmd),
        }
    }
}

impl FromStr for RunTarget {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(name) = s.strip_prefix("agent:") {
            Ok(RunTarget::Agent(name.to_string()))
        } else if let Some(cmd) = s.strip_prefix("shell:") {
            Ok(RunTarget::Shell(cmd.to_string()))
        } else if let Some(name) = s.strip_prefix("job:") {
            Ok(RunTarget::Job(name.to_string()))
        } else {
            // Bare name → Job (backward compat)
            Ok(RunTarget::Job(s.to_string()))
        }
    }
}

impl Serialize for RunTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RunTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(s.parse().unwrap())
    }
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod tests;
