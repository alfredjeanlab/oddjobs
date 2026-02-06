// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

mod job_tests {
    use super::super::*;
    use tempfile::tempdir;

    #[test]
    fn append_creates_directory_and_file() {
        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        let logger = JobLogger::new(log_dir.clone());

        logger.append("pipe-1", "init", "job created");

        let content = std::fs::read_to_string(log_dir.join("job/pipe-1.log")).unwrap();
        assert!(content.contains("[init] job created"));
    }

    #[test]
    fn multiple_appends_produce_ordered_lines() {
        let dir = tempdir().unwrap();
        let logger = JobLogger::new(dir.path().to_path_buf());

        logger.append("pipe-1", "init", "step started");
        logger.append("pipe-1", "init", "shell: echo hello");
        logger.append("pipe-1", "init", "shell completed (exit 0)");

        let content = std::fs::read_to_string(dir.path().join("job/pipe-1.log")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("[init] step started"));
        assert!(lines[1].contains("[init] shell: echo hello"));
        assert!(lines[2].contains("[init] shell completed (exit 0)"));
    }

    #[test]
    fn lines_match_expected_format() {
        let dir = tempdir().unwrap();
        let logger = JobLogger::new(dir.path().to_path_buf());

        logger.append("pipe-1", "plan", "agent spawned: planner");

        let content = std::fs::read_to_string(dir.path().join("job/pipe-1.log")).unwrap();
        let line = content.trim();

        // Format: YYYY-MM-DDTHH:MM:SSZ [step] message
        assert!(
            line.chars().nth(4) == Some('-'),
            "expected date format, got: {}",
            line
        );
        assert!(line.contains("[plan] agent spawned: planner"));
        assert_eq!(line.chars().nth(10), Some('T'));
        assert!(line.ends_with("Z [plan] agent spawned: planner"));
        assert!(
            line.len() > 20,
            "line too short for expected format: {}",
            line
        );
    }

    #[test]
    fn separate_jobs_get_separate_files() {
        let dir = tempdir().unwrap();
        let logger = JobLogger::new(dir.path().to_path_buf());

        logger.append("pipe-1", "init", "first job");
        logger.append("pipe-2", "init", "second job");

        let content1 = std::fs::read_to_string(dir.path().join("job/pipe-1.log")).unwrap();
        let content2 = std::fs::read_to_string(dir.path().join("job/pipe-2.log")).unwrap();
        assert!(content1.contains("first job"));
        assert!(content2.contains("second job"));
        assert!(!content1.contains("second job"));
    }

    #[test]
    fn bad_path_does_not_panic() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("blocker");
        std::fs::write(&file_path, "not a dir").unwrap();

        let logger = JobLogger::new(file_path.join("nested"));

        // Should not panic, just log a warning
        logger.append("pipe-1", "init", "should not panic");
    }

    #[test]
    fn agent_pointer_uses_absolute_path() {
        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        let logger = JobLogger::new(log_dir.clone());

        let agent_id = "8cf5e1df-a434-4029-a369-c95af9c374c9";
        logger.append_agent_pointer("pipe-1", "plan", agent_id);

        let content = std::fs::read_to_string(log_dir.join("job/pipe-1.log")).unwrap();
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
        let logger = JobLogger::new(log_dir.clone());

        let source_dir = dir.path().join("source");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("session.jsonl");
        std::fs::write(&source, r#"{"type":"user","message":"hello"}"#).unwrap();

        let agent_id = "8cf5e1df-a434-4029-a369-c95af9c374c9";
        logger.copy_session_log(agent_id, &source);

        let dest = log_dir.join("agent").join(agent_id).join("session.jsonl");
        assert!(dest.exists(), "session.jsonl should exist at {:?}", dest);

        let content = std::fs::read_to_string(&dest).unwrap();
        assert!(content.contains(r#"{"type":"user","message":"hello"}"#));
    }

    #[test]
    fn append_agent_error_writes_to_agent_log() {
        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        let logger = JobLogger::new(log_dir.clone());

        let agent_id = "8cf5e1df-a434-4029-a369-c95af9c374c9";
        logger.append_agent_error(agent_id, "rate limit exceeded");

        let content =
            std::fs::read_to_string(log_dir.join("agent").join(format!("{}.log", agent_id)))
                .unwrap();
        assert!(
            content.contains("error: rate limit exceeded"),
            "expected error in agent log, got: {}",
            content
        );
        assert!(content.starts_with("20"), "expected timestamp prefix");
    }

    #[test]
    fn append_agent_error_appends_multiple() {
        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        let logger = JobLogger::new(log_dir.clone());

        let agent_id = "test-agent-1";
        logger.append_agent_error(agent_id, "first error");
        logger.append_agent_error(agent_id, "second error");

        let content =
            std::fs::read_to_string(log_dir.join("agent").join(format!("{}.log", agent_id)))
                .unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("error: first error"));
        assert!(lines[1].contains("error: second error"));
    }

    #[test]
    fn copy_session_log_handles_missing_source() {
        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        let logger = JobLogger::new(log_dir.clone());

        let source = dir.path().join("nonexistent.jsonl");

        let agent_id = "abc-123";
        // Should not panic, just log a warning
        logger.copy_session_log(agent_id, &source);

        let dest_dir = log_dir.join("agent").join(agent_id);
        assert!(dest_dir.exists());
    }

    #[test]
    fn append_fenced_writes_correctly_formatted_block() {
        let dir = tempdir().unwrap();
        let logger = JobLogger::new(dir.path().to_path_buf());

        logger.append_fenced("pipe-1", "init", "stdout", "hello world\n");

        let content = std::fs::read_to_string(dir.path().join("job/pipe-1.log")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("[init] ```stdout"));
        assert_eq!(lines[1], "hello world");
        assert!(lines[2].contains("[init] ```"));
        assert!(!lines[2].contains("```stdout"));
    }

    #[test]
    fn append_fenced_adds_trailing_newline_when_missing() {
        let dir = tempdir().unwrap();
        let logger = JobLogger::new(dir.path().to_path_buf());

        logger.append_fenced("pipe-1", "build", "stderr", "warning: unused variable");

        let content = std::fs::read_to_string(dir.path().join("job/pipe-1.log")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("[build] ```stderr"));
        assert_eq!(lines[1], "warning: unused variable");
        assert!(lines[2].contains("[build] ```"));
    }

    #[test]
    fn append_fenced_multiline_content() {
        let dir = tempdir().unwrap();
        let logger = JobLogger::new(dir.path().to_path_buf());

        logger.append_fenced(
            "pipe-1",
            "build",
            "stdout",
            "Compiling oj v0.1.0\n    Finished dev target(s) in 12.34s\n",
        );

        let content = std::fs::read_to_string(dir.path().join("job/pipe-1.log")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("[build] ```stdout"));
        assert_eq!(lines[1], "Compiling oj v0.1.0");
        assert_eq!(lines[2], "    Finished dev target(s) in 12.34s");
        assert!(lines[3].contains("[build] ```"));
    }

    #[test]
    fn append_fenced_integrates_with_append() {
        let dir = tempdir().unwrap();
        let logger = JobLogger::new(dir.path().to_path_buf());

        logger.append_fenced("pipe-1", "init", "stdout", "hello world\n");
        logger.append("pipe-1", "init", "shell completed (exit 0)");

        let content = std::fs::read_to_string(dir.path().join("job/pipe-1.log")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("[init] ```stdout"));
        assert_eq!(lines[1], "hello world");
        assert!(lines[2].contains("[init] ```"));
        assert!(lines[3].contains("[init] shell completed (exit 0)"));
    }
}

mod worker_tests {
    use super::super::*;

    #[test]
    fn append_creates_log_file_and_writes_line() {
        let dir = tempfile::tempdir().unwrap();
        let logger = WorkerLogger::new(dir.path().to_path_buf());

        logger.append("test-worker", "started (queue=bugs, concurrency=2)");

        let log_path = dir.path().join("worker/test-worker.log");
        assert!(log_path.exists());

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("[worker] started (queue=bugs, concurrency=2)"));
        assert!(content.ends_with('\n'));
        // Check timestamp format: YYYY-MM-DDTHH:MM:SSZ
        let first_line = content.lines().next().unwrap();
        assert!(first_line.len() > 20);
        assert!(first_line.contains('T'));
        assert!(first_line.contains('Z'));
    }

    #[test]
    fn append_accumulates_multiple_lines() {
        let dir = tempfile::tempdir().unwrap();
        let logger = WorkerLogger::new(dir.path().to_path_buf());

        logger.append("my-worker", "started");
        logger.append("my-worker", "dispatched item abc123");
        logger.append("my-worker", "stopped");

        let log_path = dir.path().join("worker/my-worker.log");
        let content = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("started"));
        assert!(lines[1].contains("dispatched item abc123"));
        assert!(lines[2].contains("stopped"));
    }

    #[test]
    fn append_with_namespace_creates_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        let logger = WorkerLogger::new(dir.path().to_path_buf());

        logger.append("myproject/test-worker", "started");

        let log_path = dir.path().join("worker/myproject/test-worker.log");
        assert!(log_path.exists());

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("[worker] started"));
    }
}

mod queue_tests {
    use super::super::*;
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
        assert!(
            line.starts_with("20"),
            "line should start with timestamp: {}",
            line
        );
        assert!(line.contains('T'));
        assert!(line.contains('Z'));
    }
}
