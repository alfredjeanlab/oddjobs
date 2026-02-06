// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::session::{FakeSessionAdapter, SessionCall};
use oj_core::{AgentId, JobId, OwnerId};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::sync::mpsc;

#[tokio::test]
async fn spawn_rejects_nonexistent_cwd() {
    let sessions = FakeSessionAdapter::default();
    let adapter = ClaudeAgentAdapter::new(sessions);
    let (tx, _rx) = mpsc::channel(10);

    let project_dir = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    let config = AgentSpawnConfig {
        agent_id: AgentId::new("test-agent-1"),
        agent_name: "claude".to_string(),
        command: "claude code".to_string(),
        env: vec![],
        workspace_path: workspace_dir.path().to_path_buf(),
        cwd: Some(PathBuf::from("/nonexistent/path")),
        prompt: "Test prompt".to_string(),
        job_name: "test-job".to_string(),
        job_id: "pipe-1".to_string(),
        project_root: project_dir.path().to_path_buf(),
        session_config: HashMap::new(),
        owner: OwnerId::Job(JobId::default()),
    };

    let result = adapter.spawn(config, tx).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("working directory does not exist"),
        "Expected error about working directory, got: {}",
        err
    );
}

#[tokio::test]
async fn test_prepare_workspace() {
    let project_dir = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    prepare_workspace(workspace_dir.path(), project_dir.path())
        .await
        .unwrap();

    // Workspace directory should exist
    assert!(workspace_dir.path().exists());

    // Should NOT write a CLAUDE.md (prompt is passed via CLI arg)
    let claude_md = workspace_dir.path().join("CLAUDE.md");
    assert!(!claude_md.exists());
}

#[tokio::test]
async fn test_prepare_workspace_copies_settings() {
    let project_dir = TempDir::new().unwrap();
    let workspace_dir = TempDir::new().unwrap();

    // Create project settings
    let settings_dir = project_dir.path().join(".claude");
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(settings_dir.join("settings.json"), r#"{"key": "value"}"#).unwrap();

    prepare_workspace(workspace_dir.path(), project_dir.path())
        .await
        .unwrap();

    let copied_settings = workspace_dir.path().join(".claude/settings.local.json");
    assert!(copied_settings.exists());
}

#[test]
fn test_augment_command_adds_allow_flag() {
    let cmd = "claude --dangerously-skip-permissions";
    let result = augment_command_for_skip_permissions(cmd);
    assert_eq!(
        result,
        "claude --dangerously-skip-permissions --allow-dangerously-skip-permissions"
    );
}

#[test]
fn test_augment_command_no_change_when_allow_present() {
    let cmd = "claude --dangerously-skip-permissions --allow-dangerously-skip-permissions";
    let result = augment_command_for_skip_permissions(cmd);
    assert_eq!(result, cmd);
}

#[test]
fn test_augment_command_no_change_without_skip_flag() {
    let cmd = "claude --print";
    let result = augment_command_for_skip_permissions(cmd);
    assert_eq!(result, cmd);
}

#[tokio::test]
async fn test_handle_bypass_permissions_prompt_accepts() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);

    // Simulate the bypass permissions prompt output
    sessions.set_output(
        "test-session",
        vec![
            "WARNING: Claude Code running in Bypass Permissions mode".to_string(),
            "".to_string(),
            "❯ 1. No, exit".to_string(),
            "  2. Yes, I accept".to_string(),
        ],
    );

    let result = handle_bypass_permissions_prompt(&sessions, "test-session", 1)
        .await
        .unwrap();

    assert_eq!(result, BypassPromptResult::Accepted);

    // Verify "2" was sent to accept
    let calls = sessions.calls();
    let send_calls: Vec<_> = calls
        .iter()
        .filter_map(|c| match c {
            SessionCall::Send { id, input } => Some((id.clone(), input.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(send_calls.len(), 1);
    assert_eq!(send_calls[0], ("test-session".to_string(), "2".to_string()));
}

#[tokio::test]
async fn test_handle_bypass_permissions_prompt_not_present() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);

    // Simulate normal Claude startup (no prompt)
    sessions.set_output("test-session", vec!["Claude Code is ready.".to_string()]);

    let result = handle_bypass_permissions_prompt(&sessions, "test-session", 1)
        .await
        .unwrap();

    assert_eq!(result, BypassPromptResult::NotPresent);

    // Verify no send was called
    let calls = sessions.calls();
    let send_calls: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, SessionCall::Send { .. }))
        .collect();
    assert!(send_calls.is_empty());
}

#[tokio::test]
async fn test_handle_workspace_trust_prompt_accepts() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);

    // Simulate the workspace trust prompt output
    sessions.set_output(
        "test-session",
        vec![
            "Accessing workspace:".to_string(),
            "/Users/test/project".to_string(),
            "".to_string(),
            "❯ 1. Yes, I trust this folder".to_string(),
            "  2. No, exit".to_string(),
        ],
    );

    let result = handle_workspace_trust_prompt(&sessions, "test-session", 1)
        .await
        .unwrap();

    assert_eq!(result, WorkspaceTrustResult::Accepted);

    // Verify "1" was sent to trust
    let calls = sessions.calls();
    let send_calls: Vec<_> = calls
        .iter()
        .filter_map(|c| match c {
            SessionCall::Send { id, input } => Some((id.clone(), input.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(send_calls.len(), 1);
    assert_eq!(send_calls[0], ("test-session".to_string(), "1".to_string()));
}

#[tokio::test]
async fn test_handle_workspace_trust_prompt_not_present() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);

    // Simulate normal Claude startup (no prompt)
    sessions.set_output("test-session", vec!["Claude Code is ready.".to_string()]);

    let result = handle_workspace_trust_prompt(&sessions, "test-session", 1)
        .await
        .unwrap();

    assert_eq!(result, WorkspaceTrustResult::NotPresent);

    // Verify no send was called
    let calls = sessions.calls();
    let send_calls: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, SessionCall::Send { .. }))
        .collect();
    assert!(send_calls.is_empty());
}

// Session name generation tests

#[test]
fn sanitize_removes_invalid_chars() {
    assert_eq!(sanitize_for_tmux("foo:bar.baz", 20), "foo-bar-baz");
}

#[test]
fn sanitize_preserves_valid_chars() {
    assert_eq!(sanitize_for_tmux("my-job_123", 20), "my-job_123");
}

#[test]
fn sanitize_replaces_spaces() {
    assert_eq!(sanitize_for_tmux("Job With Spaces", 25), "Job-With-Spaces");
}

#[test]
fn sanitize_truncates_long_names() {
    assert_eq!(sanitize_for_tmux("very-long-name-here", 10), "very-long");
}

#[test]
fn sanitize_collapses_multiple_hyphens() {
    assert_eq!(sanitize_for_tmux("foo---bar", 20), "foo-bar");
    assert_eq!(sanitize_for_tmux("a::b..c", 20), "a-b-c");
}

#[test]
fn sanitize_handles_empty_string() {
    assert_eq!(sanitize_for_tmux("", 10), "");
}

#[test]
fn sanitize_trims_trailing_hyphen_on_truncate() {
    // "test-long-" at 10 chars would end with hyphen, should trim it
    assert_eq!(sanitize_for_tmux("test-long-name", 10), "test-long");
}

#[test]
fn session_name_has_expected_format() {
    let name = generate_session_name("my-job", "claude");
    assert!(
        name.starts_with("my-job-claude-"),
        "Expected name to start with 'my-job-claude-', got: {}",
        name
    );
    // Should have 4 hex chars at the end
    let suffix = &name["my-job-claude-".len()..];
    assert_eq!(suffix.len(), 4, "Expected 4-char suffix, got: {}", suffix);
    assert!(
        suffix.chars().all(|c| c.is_ascii_hexdigit()),
        "Expected hex suffix, got: {}",
        suffix
    );
}

#[test]
fn session_name_sanitizes_job_name() {
    let name = generate_session_name("my.dotted.name", "agent-1");
    assert!(
        name.starts_with("my-dotted-name-agent-1-"),
        "Expected sanitized job name, got: {}",
        name
    );
}

#[test]
fn session_name_truncates_long_names() {
    let name = generate_session_name("very-long-job-name-here", "implementation-step");
    // Job truncated to 20, step to 15, plus 4 random + 2 hyphens
    // Should be reasonable length
    assert!(name.len() <= 45, "Name too long: {} ({})", name, name.len());
}

#[test]
fn generate_short_random_produces_correct_length() {
    let random = generate_short_random(4);
    assert_eq!(random.len(), 4);

    let random8 = generate_short_random(8);
    assert_eq!(random8.len(), 8);
}

#[test]
fn generate_short_random_produces_hex_chars() {
    let random = generate_short_random(10);
    assert!(
        random.chars().all(|c| c.is_ascii_hexdigit()),
        "Expected all hex chars, got: {}",
        random
    );
}

#[tokio::test]
async fn test_handle_login_prompt_detected_select_login() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);

    sessions.set_output(
        "test-session",
        vec![
            "Welcome to Claude Code!".to_string(),
            "".to_string(),
            "Select login method".to_string(),
            "1. Anthropic".to_string(),
            "2. Google".to_string(),
        ],
    );

    let result = handle_login_prompt(&sessions, "test-session", 1)
        .await
        .unwrap();

    assert_eq!(result, LoginPromptResult::Detected);
}

#[tokio::test]
async fn test_handle_login_prompt_detected_text_style() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);

    sessions.set_output(
        "test-session",
        vec![
            "Welcome to Claude Code!".to_string(),
            "Choose the text style".to_string(),
        ],
    );

    let result = handle_login_prompt(&sessions, "test-session", 1)
        .await
        .unwrap();

    assert_eq!(result, LoginPromptResult::Detected);
}

#[tokio::test]
async fn test_handle_login_prompt_not_present() {
    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);

    sessions.set_output("test-session", vec!["Claude Code is ready.".to_string()]);

    let result = handle_login_prompt(&sessions, "test-session", 1)
        .await
        .unwrap();

    assert_eq!(result, LoginPromptResult::NotPresent);
}

#[tokio::test]
async fn send_clears_input_before_message() {
    use crate::agent::AgentAdapter;

    let sessions = FakeSessionAdapter::new();
    sessions.add_session("test-session", true);

    let adapter = ClaudeAgentAdapter::new(sessions.clone());
    let agent_id = AgentId::new("test-agent-1");
    adapter.register_test_agent(&agent_id, "test-session");

    adapter.send(&agent_id, "hello world").await.unwrap();

    let calls = sessions.calls();

    // Filter to only the calls made by send (skip the add_session setup)
    // Expected sequence: Send(Escape), Send(Escape), SendLiteral(text), SendEnter
    let send_calls: Vec<_> = calls
        .iter()
        .filter(|c| {
            matches!(
                c,
                SessionCall::Send { .. }
                    | SessionCall::SendLiteral { .. }
                    | SessionCall::SendEnter { .. }
            )
        })
        .collect();

    assert_eq!(
        send_calls.len(),
        4,
        "Expected 4 calls, got: {:?}",
        send_calls
    );

    // First: Escape to clear input
    assert!(
        matches!(&send_calls[0], SessionCall::Send { id, input } if id == "test-session" && input == "Escape"),
        "Expected first Escape, got: {:?}",
        send_calls[0]
    );

    // Second: Escape again
    assert!(
        matches!(&send_calls[1], SessionCall::Send { id, input } if id == "test-session" && input == "Escape"),
        "Expected second Escape, got: {:?}",
        send_calls[1]
    );

    // Third: Literal message text
    assert!(
        matches!(&send_calls[2], SessionCall::SendLiteral { id, text } if id == "test-session" && text == "hello world"),
        "Expected SendLiteral with message, got: {:?}",
        send_calls[2]
    );

    // Fourth: Enter
    assert!(
        matches!(&send_calls[3], SessionCall::SendEnter { id } if id == "test-session"),
        "Expected SendEnter, got: {:?}",
        send_calls[3]
    );
}
