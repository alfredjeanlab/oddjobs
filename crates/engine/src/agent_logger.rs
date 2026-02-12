// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Append-only logger for per-agent activity logs.
//!
//! Writes human-readable timestamped activity lines to:
//!   `<log_dir>/agent/<agent_id>.log`

use oj_adapters::agent::log_entry::{AgentLogEntry, AgentLogMessage};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// Append-only logger for per-agent activity logs.
///
/// Receives `(AgentId, Vec<AgentLogEntry>)` tuples over a channel and writes
/// formatted lines to `<log_dir>/agent/<agent_id>.log`.
pub struct AgentLogger {
    log_dir: PathBuf,
}

impl AgentLogger {
    pub fn new(log_dir: PathBuf) -> Self {
        Self { log_dir }
    }

    /// Append formatted entries to the agent log file.
    ///
    /// Failures are logged via tracing but do not propagate — logging
    /// must not break the engine.
    pub fn append_entries(&self, agent_id: &str, entries: &[AgentLogEntry]) {
        if entries.is_empty() {
            return;
        }

        let path = self.log_path(agent_id);
        let Some(agent_dir) = path.parent() else {
            return;
        };

        if let Err(e) = self.write_entries(agent_dir, &path, entries) {
            tracing::warn!(agent_id, error = %e, "failed to write agent log");
        }
    }

    /// Return the path to an agent's log file.
    pub fn log_path(&self, agent_id: &str) -> PathBuf {
        self.log_dir.join("agent").join(format!("{}.log", agent_id))
    }

    fn write_entries(
        &self,
        agent_dir: &std::path::Path,
        path: &std::path::Path,
        entries: &[AgentLogEntry],
    ) -> std::io::Result<()> {
        fs::create_dir_all(agent_dir)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        for entry in entries {
            writeln!(file, "{}", entry)?;
        }
        Ok(())
    }

    /// Spawn a background task that reads from the channel and writes entries.
    ///
    /// Returns a join handle for the task.
    pub fn spawn_writer(
        log_dir: PathBuf,
        mut rx: mpsc::Receiver<AgentLogMessage>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let logger = AgentLogger::new(log_dir);
            while let Some((agent_id, entries)) = rx.recv().await {
                logger.append_entries(agent_id.as_str(), &entries);
            }
        })
    }
}

#[cfg(test)]
#[path = "agent_logger_tests.rs"]
mod tests;
