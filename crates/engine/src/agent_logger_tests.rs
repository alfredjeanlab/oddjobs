// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_adapters::agent::log_entry::{AgentLogEntry, EntryKind};
use tempfile::TempDir;

#[test]
fn append_creates_agent_directory_and_file() {
    let dir = TempDir::new().unwrap();
    let logger = AgentLogger::new(dir.path().to_path_buf());

    let entries = vec![AgentLogEntry {
        timestamp: "2026-01-30T08:17:05Z".to_string(),
        kind: EntryKind::FileRead {
            path: "/src/main.rs".to_string(),
        },
    }];

    // agent_id is a UUID
    let agent_id = "8cf5e1df-a434-4029-a369-c95af9c374c9";
    logger.append_entries(agent_id, &entries);

    // Structure: agent/{agent_id}.log
    let log_path = dir
        .path()
        .join("agent/8cf5e1df-a434-4029-a369-c95af9c374c9.log");
    assert!(log_path.exists());

    let content = std::fs::read_to_string(&log_path).unwrap();
    assert_eq!(content, "2026-01-30T08:17:05Z read: /src/main.rs\n");
}

#[test]
fn append_multiple_entries() {
    let dir = TempDir::new().unwrap();
    let logger = AgentLogger::new(dir.path().to_path_buf());

    let entries = vec![
        AgentLogEntry {
            timestamp: "2026-01-30T08:17:05Z".to_string(),
            kind: EntryKind::FileRead {
                path: "/src/main.rs".to_string(),
            },
        },
        AgentLogEntry {
            timestamp: "2026-01-30T08:17:12Z".to_string(),
            kind: EntryKind::FileWrite {
                path: "/src/lib.rs".to_string(),
                new: true,
                lines: 15,
            },
        },
        AgentLogEntry {
            timestamp: "2026-01-30T08:17:30Z".to_string(),
            kind: EntryKind::BashCommand {
                command: "cargo build".to_string(),
                exit_code: Some(0),
            },
        },
    ];

    let agent_id = "abc-123-def";
    logger.append_entries(agent_id, &entries);

    let content = std::fs::read_to_string(logger.log_path(agent_id)).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "2026-01-30T08:17:05Z read: /src/main.rs");
    assert_eq!(
        lines[1],
        "2026-01-30T08:17:12Z wrote: /src/lib.rs (new, 15 lines)"
    );
    assert_eq!(lines[2], "2026-01-30T08:17:30Z bash: cargo build (exit 0)");
}

#[test]
fn append_is_incremental() {
    let dir = TempDir::new().unwrap();
    let logger = AgentLogger::new(dir.path().to_path_buf());

    let agent_id = "xyz-agent-id";

    let entries1 = vec![AgentLogEntry {
        timestamp: "2026-01-30T08:17:05Z".to_string(),
        kind: EntryKind::FileRead {
            path: "/a.rs".to_string(),
        },
    }];
    logger.append_entries(agent_id, &entries1);

    let entries2 = vec![AgentLogEntry {
        timestamp: "2026-01-30T08:17:10Z".to_string(),
        kind: EntryKind::FileRead {
            path: "/b.rs".to_string(),
        },
    }];
    logger.append_entries(agent_id, &entries2);

    let content = std::fs::read_to_string(logger.log_path(agent_id)).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "2026-01-30T08:17:05Z read: /a.rs");
    assert_eq!(lines[1], "2026-01-30T08:17:10Z read: /b.rs");
}

#[test]
fn append_empty_entries_is_noop() {
    let dir = TempDir::new().unwrap();
    let logger = AgentLogger::new(dir.path().to_path_buf());
    logger.append_entries("some-agent-id", &[]);

    // Directory should not be created for empty entries
    assert!(!dir.path().join("agent").exists());
}

#[test]
fn log_path_returns_expected_path() {
    let logger = AgentLogger::new(PathBuf::from("/state/logs"));
    // agent_id is a UUID
    assert_eq!(
        logger.log_path("8cf5e1df-a434-4029-a369-c95af9c374c9"),
        PathBuf::from("/state/logs/agent/8cf5e1df-a434-4029-a369-c95af9c374c9.log")
    );
}
