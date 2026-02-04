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

    let agent_id = "test-agent-123";
    let prime_paths = HashMap::new();
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), &prime_paths, state_dir.path()).unwrap();

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
}

#[test]
fn prepare_agent_settings_injects_stop_hook() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    let agent_id = "test-agent-456";
    let prime_paths = HashMap::new();
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), &prime_paths, state_dir.path()).unwrap();

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
}

#[test]
fn prepare_agent_settings_merges_project_settings() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    // Create project settings
    let settings_dir = workspace.path().join(".claude");
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(settings_dir.join("settings.json"), r#"{"key": "value"}"#).unwrap();

    let agent_id = "test-agent-789";
    let prime_paths = HashMap::new();
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), &prime_paths, state_dir.path()).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // Original key is preserved
    assert_eq!(parsed["key"], "value");
    // Stop hook is also present
    assert!(parsed["hooks"]["Stop"].is_array());
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

    let agent_id = "test-agent-abc";
    let prime_paths = HashMap::new();
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), &prime_paths, state_dir.path()).unwrap();

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
}

#[test]
fn prepare_agent_prime_writes_script_for_string_form() {
    let state_dir = TempDir::new().unwrap();

    let prime = PrimeDef::Script("echo hello\ngit status".to_string());
    let vars = HashMap::new();
    let prime_paths = prepare_agent_prime("test-prime-1", &prime, &vars, state_dir.path()).unwrap();

    assert_eq!(prime_paths.len(), 1);
    let prime_path = &prime_paths[""];
    let content = fs::read_to_string(prime_path).unwrap();
    assert!(content.starts_with("#!/usr/bin/env bash\n"));
    assert!(content.contains("set -euo pipefail"));
    assert!(content.contains("echo hello\ngit status"));
}

#[test]
fn prepare_agent_prime_writes_script_for_array_form() {
    let state_dir = TempDir::new().unwrap();

    let prime = PrimeDef::Commands(vec![
        "echo hello".to_string(),
        "git status --short".to_string(),
    ]);
    let vars = HashMap::new();
    let prime_paths = prepare_agent_prime("test-prime-2", &prime, &vars, state_dir.path()).unwrap();

    assert_eq!(prime_paths.len(), 1);
    let content = fs::read_to_string(&prime_paths[""]).unwrap();
    assert!(content.contains("echo hello\ngit status --short"));
}

#[test]
fn prepare_agent_prime_sets_executable() {
    let state_dir = TempDir::new().unwrap();

    let prime = PrimeDef::Script("echo test".to_string());
    let vars = HashMap::new();
    let prime_paths = prepare_agent_prime("test-prime-3", &prime, &vars, state_dir.path()).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::metadata(&prime_paths[""]).unwrap().permissions();
        assert_eq!(perms.mode() & 0o755, 0o755);
    }
}

#[test]
fn prepare_agent_settings_with_prime_injects_session_start_hook() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    let prime = PrimeDef::Script("echo hello".to_string());
    let vars = HashMap::new();
    let prime_paths = prepare_agent_prime("test-prime-4", &prime, &vars, state_dir.path()).unwrap();

    let agent_id = "test-prime-4";
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), &prime_paths, state_dir.path()).unwrap();

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
}

#[test]
fn prepare_agent_settings_without_prime_no_session_start() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    let agent_id = "test-no-prime";
    let prime_paths = HashMap::new();
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), &prime_paths, state_dir.path()).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // No SessionStart hook
    assert!(parsed["hooks"]["SessionStart"].is_null());
    // Stop hook is present
    assert!(parsed["hooks"]["Stop"].is_array());
}

#[test]
fn prepare_agent_settings_injects_notification_hooks() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    let agent_id = "test-notif-hooks";
    let prime_paths = HashMap::new();
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), &prime_paths, state_dir.path()).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // Notification hook is present (single entry with combined matcher)
    assert!(parsed["hooks"]["Notification"].is_array());
    let notif_hooks = parsed["hooks"]["Notification"].as_array().unwrap();
    assert_eq!(notif_hooks.len(), 1);

    // Combined matcher for idle_prompt and permission_prompt
    assert_eq!(notif_hooks[0]["matcher"], "idle_prompt|permission_prompt");
    let inner_hooks = notif_hooks[0]["hooks"].as_array().unwrap();
    assert_eq!(inner_hooks.len(), 1);
    assert_eq!(inner_hooks[0]["type"], "command");
    let cmd = inner_hooks[0]["command"].as_str().unwrap();
    assert_eq!(cmd, format!("oj agent hook notify --agent-id {}", agent_id));

    // Stop hook is still present
    assert!(parsed["hooks"]["Stop"].is_array());
}

#[test]
fn prepare_agent_settings_injects_pretooluse_hook() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    let agent_id = "test-pretooluse";
    let prime_paths = HashMap::new();
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), &prime_paths, state_dir.path()).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // PreToolUse hook is present
    assert!(parsed["hooks"]["PreToolUse"].is_array());
    let hooks = parsed["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(hooks.len(), 1);

    // Matcher covers all three tools
    assert_eq!(
        hooks[0]["matcher"],
        "ExitPlanMode|AskUserQuestion|EnterPlanMode"
    );

    // Command references the agent hook subcommand
    let inner = hooks[0]["hooks"].as_array().unwrap();
    assert_eq!(inner.len(), 1);
    assert_eq!(inner[0]["type"], "command");
    assert_eq!(
        inner[0]["command"],
        format!("oj agent hook pretooluse {}", agent_id)
    );

    // Other hooks are still present
    assert!(parsed["hooks"]["Stop"].is_array());
    assert!(parsed["hooks"]["Notification"].is_array());
}

#[test]
fn prepare_agent_prime_per_source_writes_multiple_scripts() {
    let state_dir = TempDir::new().unwrap();

    let mut map = std::collections::HashMap::new();
    map.insert(
        "startup".to_string(),
        PrimeDef::Commands(vec!["echo startup".to_string(), "git status".to_string()]),
    );
    map.insert(
        "resume".to_string(),
        PrimeDef::Script("echo resume".to_string()),
    );
    let prime = PrimeDef::PerSource(map);
    let vars = HashMap::new();

    let prime_paths =
        prepare_agent_prime("test-per-source-1", &prime, &vars, state_dir.path()).unwrap();

    assert_eq!(prime_paths.len(), 2);

    // Check startup script
    let startup_path = &prime_paths["startup"];
    assert!(
        startup_path.ends_with("prime-startup.sh"),
        "startup path: {:?}",
        startup_path
    );
    let startup_content = fs::read_to_string(startup_path).unwrap();
    assert!(startup_content.contains("echo startup\ngit status"));

    // Check resume script
    let resume_path = &prime_paths["resume"];
    assert!(
        resume_path.ends_with("prime-resume.sh"),
        "resume path: {:?}",
        resume_path
    );
    let resume_content = fs::read_to_string(resume_path).unwrap();
    assert!(resume_content.contains("echo resume"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in prime_paths.values() {
            let perms = fs::metadata(path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o755, 0o755);
        }
    }
}

#[test]
fn prepare_agent_settings_per_source_injects_multiple_session_start_hooks() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    let mut map = std::collections::HashMap::new();
    map.insert(
        "startup".to_string(),
        PrimeDef::Commands(vec!["echo startup".to_string()]),
    );
    map.insert(
        "resume".to_string(),
        PrimeDef::Script("echo resume".to_string()),
    );
    let prime = PrimeDef::PerSource(map);
    let vars = HashMap::new();

    let agent_id = "test-per-source-settings";
    let prime_paths = prepare_agent_prime(agent_id, &prime, &vars, state_dir.path()).unwrap();
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), &prime_paths, state_dir.path()).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // SessionStart hooks are present
    assert!(parsed["hooks"]["SessionStart"].is_array());
    let session_start = parsed["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(session_start.len(), 2);

    // Collect matchers and commands
    let mut matchers: Vec<String> = session_start
        .iter()
        .map(|e| e["matcher"].as_str().unwrap().to_string())
        .collect();
    matchers.sort();
    assert_eq!(matchers, vec!["resume", "startup"]);

    // Each entry references the correct prime script
    for entry in session_start {
        let matcher = entry["matcher"].as_str().unwrap();
        let inner = entry["hooks"].as_array().unwrap();
        let cmd = inner[0]["command"].as_str().unwrap();
        assert!(
            cmd.contains(&format!("prime-{}.sh", matcher)),
            "command for '{}' should reference prime-{}.sh: {}",
            matcher,
            matcher,
            cmd
        );
    }
}

#[test]
fn prepare_agent_settings_empty_prime_paths_no_session_start() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    let agent_id = "test-empty-prime-paths";
    let prime_paths = HashMap::new();
    let settings_path =
        prepare_agent_settings(agent_id, workspace.path(), &prime_paths, state_dir.path()).unwrap();

    let content = fs::read_to_string(&settings_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    // No SessionStart hook
    assert!(parsed["hooks"]["SessionStart"].is_null());
}

#[test]
fn prepare_agent_prime_backward_compat_single_script() {
    let state_dir = TempDir::new().unwrap();

    // Script form produces single entry with empty matcher
    let prime = PrimeDef::Script("echo test".to_string());
    let vars = HashMap::new();
    let prime_paths =
        prepare_agent_prime("test-compat-1", &prime, &vars, state_dir.path()).unwrap();

    assert_eq!(prime_paths.len(), 1);
    assert!(prime_paths.contains_key(""));
    assert!(prime_paths[""].ends_with("prime.sh"));

    // Commands form also produces single entry with empty matcher
    let prime = PrimeDef::Commands(vec!["echo a".to_string(), "echo b".to_string()]);
    let prime_paths =
        prepare_agent_prime("test-compat-2", &prime, &vars, state_dir.path()).unwrap();

    assert_eq!(prime_paths.len(), 1);
    assert!(prime_paths.contains_key(""));
    assert!(prime_paths[""].ends_with("prime.sh"));
}
