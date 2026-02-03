// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use tempfile::TempDir;

fn setup() -> (TempDir, QueueLogger) {
    let dir = TempDir::new().unwrap();
    let logger = QueueLogger::new(dir.path().to_path_buf());
    (dir, logger)
}

#[test]
fn creates_log_file_on_first_append() {
    let (dir, logger) = setup();
    logger.append(
        "build-queue",
        "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        "pushed data={url=https://example.com}",
    );

    let path = dir.path().join("queue/build-queue.log");
    assert!(path.exists(), "log file should be created");

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("[a1b2c3d4]"));
    assert!(content.contains("pushed data={url=https://example.com}"));
}

#[test]
fn appends_multiple_entries() {
    let (dir, logger) = setup();
    let item_id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
    logger.append("q", item_id, "pushed");
    logger.append("q", item_id, "dispatched worker=my-worker");
    logger.append("q", item_id, "completed");

    let path = dir.path().join("queue/q.log");
    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("pushed"));
    assert!(lines[1].contains("dispatched worker=my-worker"));
    assert!(lines[2].contains("completed"));
}

#[test]
fn handles_namespaced_queue_name() {
    let (dir, logger) = setup();
    logger.append(
        "myproject/build-queue",
        "abcdef01-2345-6789-abcd-ef0123456789",
        "pushed",
    );

    let path = dir.path().join("queue/myproject/build-queue.log");
    assert!(path.exists(), "namespaced log file should be created");
}

#[test]
fn truncates_item_id_prefix() {
    let (dir, logger) = setup();
    logger.append("q", "abcdef0123456789", "pushed");

    let path = dir.path().join("queue/q.log");
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("[abcdef01]"));
}

#[test]
fn handles_short_item_id() {
    let (dir, logger) = setup();
    logger.append("q", "abc", "pushed");

    let path = dir.path().join("queue/q.log");
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("[abc]"));
}

#[test]
fn log_line_format() {
    let (dir, logger) = setup();
    logger.append("q", "a1b2c3d4-full-id", "failed error=\"timeout exceeded\"");

    let path = dir.path().join("queue/q.log");
    let content = std::fs::read_to_string(&path).unwrap();
    let line = content.lines().next().unwrap();

    // Format: YYYY-MM-DDTHH:MM:SSZ [prefix] message
    assert!(line.ends_with("[a1b2c3d4] failed error=\"timeout exceeded\""));
    // Verify timestamp format (starts with a year)
    assert!(
        line.starts_with("20"),
        "line should start with timestamp: {}",
        line
    );
    assert!(line.contains('T'));
    assert!(line.contains('Z'));
}
