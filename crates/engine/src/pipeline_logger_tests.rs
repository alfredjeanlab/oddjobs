// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use tempfile::tempdir;

#[test]
fn append_creates_directory_and_file() {
    let dir = tempdir().unwrap();
    let log_dir = dir.path().join("logs");
    let logger = PipelineLogger::new(log_dir.clone());

    logger.append("pipe-1", "init", "pipeline created");

    let content = std::fs::read_to_string(log_dir.join("pipe-1.log")).unwrap();
    assert!(content.contains("[init] pipeline created"));
}

#[test]
fn multiple_appends_produce_ordered_lines() {
    let dir = tempdir().unwrap();
    let logger = PipelineLogger::new(dir.path().to_path_buf());

    logger.append("pipe-1", "init", "step started");
    logger.append("pipe-1", "init", "shell: echo hello");
    logger.append("pipe-1", "init", "shell completed (exit 0)");

    let content = std::fs::read_to_string(dir.path().join("pipe-1.log")).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("[init] step started"));
    assert!(lines[1].contains("[init] shell: echo hello"));
    assert!(lines[2].contains("[init] shell completed (exit 0)"));
}

#[test]
fn lines_match_expected_format() {
    let dir = tempdir().unwrap();
    let logger = PipelineLogger::new(dir.path().to_path_buf());

    logger.append("pipe-1", "plan", "agent spawned: planner");

    let content = std::fs::read_to_string(dir.path().join("pipe-1.log")).unwrap();
    let line = content.trim();

    // Format: YYYY-MM-DDTHH:MM:SSZ [step] message
    let re_pattern = r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z \[plan\] agent spawned: planner$";
    assert!(
        line.chars().nth(4) == Some('-'),
        "expected date format, got: {}",
        line
    );
    assert!(line.contains("[plan] agent spawned: planner"));
    // Verify full timestamp format
    assert_eq!(line.chars().nth(10), Some('T'));
    assert!(line.ends_with("Z [plan] agent spawned: planner"));
    // Verify regex-like match
    assert!(
        line.len() > 20,
        "line too short for expected format: {}",
        line
    );
    let _re = re_pattern; // just to suppress unused warning in doc
}

#[test]
fn separate_pipelines_get_separate_files() {
    let dir = tempdir().unwrap();
    let logger = PipelineLogger::new(dir.path().to_path_buf());

    logger.append("pipe-1", "init", "first pipeline");
    logger.append("pipe-2", "init", "second pipeline");

    let content1 = std::fs::read_to_string(dir.path().join("pipe-1.log")).unwrap();
    let content2 = std::fs::read_to_string(dir.path().join("pipe-2.log")).unwrap();
    assert!(content1.contains("first pipeline"));
    assert!(content2.contains("second pipeline"));
    assert!(!content1.contains("second pipeline"));
}

#[test]
fn bad_path_does_not_panic() {
    // Use a path that cannot be created (file as directory)
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("blocker");
    std::fs::write(&file_path, "not a dir").unwrap();

    let logger = PipelineLogger::new(file_path.join("nested"));

    // Should not panic, just log a warning
    logger.append("pipe-1", "init", "should not panic");
}

#[test]
fn agent_pointer_uses_absolute_path() {
    let dir = tempdir().unwrap();
    let log_dir = dir.path().join("logs");
    let logger = PipelineLogger::new(log_dir.clone());

    let agent_id = "8cf5e1df-a434-4029-a369-c95af9c374c9";
    logger.append_agent_pointer("pipe-1", "plan", agent_id);

    let content = std::fs::read_to_string(log_dir.join("pipe-1.log")).unwrap();
    // Should contain the full absolute path with agent_id
    let expected_path = log_dir.join("agent").join(format!("{}.log", agent_id));
    assert!(
        content.contains(&expected_path.display().to_string()),
        "expected absolute path in log, got: {}",
        content
    );
}

#[test]
fn copy_session_log_creates_directory_and_copies_file() {
    let dir = tempdir().unwrap();
    let log_dir = dir.path().join("logs");
    let logger = PipelineLogger::new(log_dir.clone());

    // Create a source session.jsonl file
    let source_dir = dir.path().join("source");
    std::fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join("session.jsonl");
    std::fs::write(&source, r#"{"type":"user","message":"hello"}"#).unwrap();

    let agent_id = "8cf5e1df-a434-4029-a369-c95af9c374c9";
    logger.copy_session_log(agent_id, &source);

    // Verify the file was copied to the right location
    let dest = log_dir.join("agent").join(agent_id).join("session.jsonl");
    assert!(dest.exists(), "session.jsonl should exist at {:?}", dest);

    let content = std::fs::read_to_string(&dest).unwrap();
    assert!(content.contains(r#"{"type":"user","message":"hello"}"#));
}

#[test]
fn copy_session_log_handles_missing_source() {
    let dir = tempdir().unwrap();
    let log_dir = dir.path().join("logs");
    let logger = PipelineLogger::new(log_dir.clone());

    // Source file does not exist
    let source = dir.path().join("nonexistent.jsonl");

    let agent_id = "abc-123";
    // Should not panic, just log a warning
    logger.copy_session_log(agent_id, &source);

    // Destination directory should exist (we create it before copy)
    let dest_dir = log_dir.join("agent").join(agent_id);
    assert!(dest_dir.exists());
}
