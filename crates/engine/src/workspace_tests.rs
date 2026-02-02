// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_runbook::PrimeDef;
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

#[test]
fn prepare_for_agent_creates_directories() {
    let workspace = TempDir::new().unwrap();

    prepare_for_agent(workspace.path()).unwrap();

    // Workspace directory exists
    assert!(workspace.path().exists());
    // .claude directory exists
    assert!(workspace.path().join(".claude").exists());
    // Settings are NOT written (they go to OJ state dir now)
    assert!(!workspace
        .path()
        .join(".claude/settings.local.json")
        .exists());
}

#[test]
fn prepare_does_not_overwrite_claude_md() {
    let workspace = TempDir::new().unwrap();

    // Simulate a project CLAUDE.md already in the workspace
    let claude_md = workspace.path().join("CLAUDE.md");
    fs::write(&claude_md, "# Project Policies\nDo not overwrite me.\n").unwrap();

    prepare_for_agent(workspace.path()).unwrap();

    let content = fs::read_to_string(&claude_md).unwrap();
    assert_eq!(content, "# Project Policies\nDo not overwrite me.\n");
}

#[test]
fn prepare_agent_settings_creates_file_in_state_dir() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    // Set OJ_STATE_DIR for this test
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let agent_id = "test-agent-123";
    let settings_path = prepare_agent_settings(agent_id, workspace.path(), None).unwrap();

    // Settings file should be in state dir
    assert!(settings_path.starts_with(state_dir.path()));
    assert!(settings_path.exists());

    // Verify correct path structure
    let expected_path = state_dir
        .path()
        .join("agents")
        .join(agent_id)
        .join("claude-settings.json");
    assert_eq!(settings_path, expected_path);

    // Clean up env var
    std::env::remove_var("OJ_STATE_DIR");
}

#[test]
fn prepare_agent_settings_injects_stop_hook() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let agent_id = "test-agent-456";
    let settings_path = prepare_agent_settings(agent_id, workspace.path(), None).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // Stop hook is present with agent_id baked in
    assert!(parsed["hooks"]["Stop"].is_array());
    let stop_hooks = parsed["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop_hooks.len(), 1);

    // Verify nested structure: matcher + hooks array
    assert_eq!(stop_hooks[0]["matcher"], "");
    let inner_hooks = stop_hooks[0]["hooks"].as_array().unwrap();
    assert_eq!(inner_hooks.len(), 1);
    assert_eq!(inner_hooks[0]["type"], "command");
    assert_eq!(
        inner_hooks[0]["command"],
        format!("oj agent hook stop {}", agent_id)
    );

    std::env::remove_var("OJ_STATE_DIR");
}

#[test]
fn prepare_agent_settings_merges_project_settings() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    // Create project settings
    let settings_dir = workspace.path().join(".claude");
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(settings_dir.join("settings.json"), r#"{"key": "value"}"#).unwrap();

    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let agent_id = "test-agent-789";
    let settings_path = prepare_agent_settings(agent_id, workspace.path(), None).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // Original key is preserved
    assert_eq!(parsed["key"], "value");
    // Stop hook is also present
    assert!(parsed["hooks"]["Stop"].is_array());

    std::env::remove_var("OJ_STATE_DIR");
}

#[test]
fn prepare_agent_settings_overwrites_existing_stop_hook() {
    // Unlike the old behavior, the new function always sets the Stop hook
    // because we control this settings file entirely
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    // Create project settings with an existing Stop hook
    let settings_dir = workspace.path().join(".claude");
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(
        settings_dir.join("settings.json"),
        r#"{"hooks": {"Stop": [{"type": "command", "command": "custom-hook"}]}}"#,
    )
    .unwrap();

    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let agent_id = "test-agent-abc";
    let settings_path = prepare_agent_settings(agent_id, workspace.path(), None).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // Stop hook is overwritten with our hook (we control this settings file)
    let stop_hooks = parsed["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop_hooks.len(), 1);
    let inner_hooks = stop_hooks[0]["hooks"].as_array().unwrap();
    assert!(inner_hooks[0]["command"]
        .as_str()
        .unwrap()
        .contains("oj agent hook stop"));

    std::env::remove_var("OJ_STATE_DIR");
}

#[test]
fn prepare_agent_prime_writes_script_for_string_form() {
    let state_dir = TempDir::new().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let prime = PrimeDef::Script("echo hello\ngit status".to_string());
    let vars = HashMap::new();
    let prime_path = prepare_agent_prime("test-prime-1", &prime, &vars).unwrap();

    let content = fs::read_to_string(&prime_path).unwrap();
    assert!(content.starts_with("#!/usr/bin/env bash\n"));
    assert!(content.contains("set -euo pipefail"));
    assert!(content.contains("echo hello\ngit status"));

    std::env::remove_var("OJ_STATE_DIR");
}

#[test]
fn prepare_agent_prime_writes_script_for_array_form() {
    let state_dir = TempDir::new().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let prime = PrimeDef::Commands(vec![
        "echo hello".to_string(),
        "git status --short".to_string(),
    ]);
    let vars = HashMap::new();
    let prime_path = prepare_agent_prime("test-prime-2", &prime, &vars).unwrap();

    let content = fs::read_to_string(&prime_path).unwrap();
    assert!(content.contains("echo hello\ngit status --short"));

    std::env::remove_var("OJ_STATE_DIR");
}

#[test]
fn prepare_agent_prime_sets_executable() {
    let state_dir = TempDir::new().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let prime = PrimeDef::Script("echo test".to_string());
    let vars = HashMap::new();
    let prime_path = prepare_agent_prime("test-prime-3", &prime, &vars).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::metadata(&prime_path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o755, 0o755);
    }

    std::env::remove_var("OJ_STATE_DIR");
}

#[test]
fn prepare_agent_settings_with_prime_injects_session_start_hook() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let prime = PrimeDef::Script("echo hello".to_string());
    let vars = HashMap::new();
    let prime_path = prepare_agent_prime("test-prime-4", &prime, &vars).unwrap();

    let agent_id = "test-prime-4";
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), Some(&prime_path)).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // SessionStart hook is present
    assert!(parsed["hooks"]["SessionStart"].is_array());
    let session_start = parsed["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(session_start.len(), 1);
    let inner_hooks = session_start[0]["hooks"].as_array().unwrap();
    assert_eq!(inner_hooks[0]["type"], "command");
    let cmd = inner_hooks[0]["command"].as_str().unwrap();
    assert!(
        cmd.contains("bash") && cmd.contains("prime.sh"),
        "SessionStart command should reference prime.sh: {}",
        cmd
    );

    // Stop hook is still present
    assert!(parsed["hooks"]["Stop"].is_array());

    std::env::remove_var("OJ_STATE_DIR");
}

#[test]
fn prepare_agent_settings_without_prime_no_session_start() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let agent_id = "test-no-prime";
    let settings_path = prepare_agent_settings(agent_id, workspace.path(), None).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // No SessionStart hook
    assert!(parsed["hooks"]["SessionStart"].is_null());
    // Stop hook is present
    assert!(parsed["hooks"]["Stop"].is_array());

    std::env::remove_var("OJ_STATE_DIR");
}

#[test]
fn prepare_agent_settings_injects_notification_hooks() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    std::env::set_var("OJ_STATE_DIR", state_dir.path());

    let agent_id = "test-notif-hooks";
    let settings_path = prepare_agent_settings(agent_id, workspace.path(), None).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // Notification hooks are present
    assert!(parsed["hooks"]["Notification"].is_array());
    let notif_hooks = parsed["hooks"]["Notification"].as_array().unwrap();
    assert_eq!(notif_hooks.len(), 2);

    // First hook: idle_prompt matcher
    assert_eq!(notif_hooks[0]["matcher"], "idle_prompt");
    let idle_hooks = notif_hooks[0]["hooks"].as_array().unwrap();
    assert_eq!(idle_hooks.len(), 1);
    assert_eq!(idle_hooks[0]["type"], "command");
    let idle_cmd = idle_hooks[0]["command"].as_str().unwrap();
    assert_eq!(idle_cmd, format!("oj emit agent:idle --agent {}", agent_id));

    // Second hook: permission_prompt matcher
    assert_eq!(notif_hooks[1]["matcher"], "permission_prompt");
    let perm_hooks = notif_hooks[1]["hooks"].as_array().unwrap();
    assert_eq!(perm_hooks.len(), 1);
    assert_eq!(perm_hooks[0]["type"], "command");
    let perm_cmd = perm_hooks[0]["command"].as_str().unwrap();
    assert_eq!(
        perm_cmd,
        format!(
            "oj emit agent:prompt --agent {} --type permission",
            agent_id
        )
    );

    // Stop hook is still present
    assert!(parsed["hooks"]["Stop"].is_array());

    std::env::remove_var("OJ_STATE_DIR");
}
