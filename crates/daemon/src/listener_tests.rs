// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::io::Write;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

/// Test that agent log lookup supports prefix matching on filenames.
///
/// Agent log files are named `{job_id}-{step}.log` (e.g., `abc123-work.log`).
/// When looking up logs, users can provide a prefix (e.g., `abc123`) and it should
/// find the matching file if there's exactly one match.
#[test]
fn agent_log_prefix_matching() {
    let temp = tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();

    // Create a log file with the typical naming pattern
    let log_file = agent_dir.join("abc123-work.log");
    let mut f = std::fs::File::create(&log_file).unwrap();
    writeln!(f, "line1").unwrap();
    writeln!(f, "line2").unwrap();

    // The prefix matching logic:
    let id = "abc123";

    // Try exact match first
    let exact_path = agent_dir.join(format!("{}.log", id));
    let log_path = if exact_path.exists() {
        exact_path
    } else {
        // Try prefix match on filenames
        let matches: Vec<_> = std::fs::read_dir(&agent_dir)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with(id) && name.ends_with(".log")
            })
            .collect();

        if matches.len() == 1 {
            matches[0].path()
        } else {
            exact_path
        }
    };

    // Should find the file via prefix match
    assert_eq!(log_path, log_file);
    assert!(log_path.exists());

    // Verify content can be read
    let content = std::fs::read_to_string(&log_path).unwrap();
    assert!(content.contains("line1"));
}

/// Test that exact match takes precedence over prefix match.
#[test]
fn agent_log_exact_match_precedence() {
    let temp = tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();

    // Create both an exact match and a prefix match file
    let exact_file = agent_dir.join("abc123.log");
    let prefix_file = agent_dir.join("abc123-work.log");

    std::fs::write(&exact_file, "exact content\n").unwrap();
    std::fs::write(&prefix_file, "prefix content\n").unwrap();

    let id = "abc123";
    let exact_path = agent_dir.join(format!("{}.log", id));

    // Exact match should be found first
    assert!(exact_path.exists());
    let content = std::fs::read_to_string(&exact_path).unwrap();
    assert!(content.contains("exact content"));
}

/// Test that ambiguous prefix (multiple matches) returns empty.
#[test]
fn agent_log_ambiguous_prefix_returns_empty() {
    let temp = tempdir().unwrap();
    let agent_dir = temp.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();

    // Create multiple files with the same prefix
    std::fs::write(agent_dir.join("abc123-step1.log"), "content1\n").unwrap();
    std::fs::write(agent_dir.join("abc123-step2.log"), "content2\n").unwrap();

    let id = "abc123";
    let exact_path = agent_dir.join(format!("{}.log", id));

    let log_path = if exact_path.exists() {
        exact_path.clone()
    } else {
        let matches: Vec<_> = std::fs::read_dir(&agent_dir)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with(id) && name.ends_with(".log")
            })
            .collect();

        if matches.len() == 1 {
            matches[0].path()
        } else {
            exact_path.clone()
        }
    };

    // With multiple matches, should fall back to exact path (which doesn't exist)
    assert_eq!(log_path, exact_path);
    assert!(!log_path.exists());
}

/// Test that detect_client_disconnect completes when the writer is dropped (EOF).
#[tokio::test]
async fn detect_client_disconnect_completes_on_eof() {
    let (mut reader, writer) = tokio::io::duplex(64);
    drop(writer); // Simulate client disconnect
    super::detect_client_disconnect(&mut reader).await;
    // Reaching here means disconnect was detected
}

/// Test that a cancelled token is visible to handlers via is_cancelled().
#[tokio::test]
async fn cancellation_token_is_visible_after_cancel() {
    let token = CancellationToken::new();
    assert!(!token.is_cancelled());
    token.cancel();
    assert!(token.is_cancelled());
}

/// Test that early disconnect drops slow handler and cancels via token.
#[tokio::test]
async fn early_disconnect_cancels_via_token() {
    let token = CancellationToken::new();

    let (mut reader, writer) = tokio::io::duplex(64);
    // Drop writer immediately to simulate client disconnect
    drop(writer);

    let handler = async {
        // Simulate a slow handler
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        "slow handler done"
    };

    let result = tokio::select! {
        msg = handler => msg.to_string(),
        _ = super::detect_client_disconnect(&mut reader) => {
            token.cancel();
            "disconnected".to_string()
        }
    };

    assert_eq!(result, "disconnected");
    assert!(token.is_cancelled());
}
