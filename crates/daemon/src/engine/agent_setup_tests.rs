// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_runbook::PrimeDef;
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

/// Helper: write agent-config with defaults and return parsed JSON.
fn config_for(
    agent_id: &str,
    workspace: &Path,
    prime: Option<&PrimeDef>,
    prime_vars: &HashMap<String, String>,
    on_idle: &str,
    on_idle_message: Option<&str>,
) -> (TempDir, serde_json::Value) {
    let state_dir = TempDir::new().unwrap();
    let config_path = write_agent_config_file(
        agent_id,
        workspace,
        prime,
        prime_vars,
        on_idle,
        on_idle_message,
        state_dir.path(),
    )
    .unwrap();
    let content = fs::read_to_string(&config_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    (state_dir, parsed)
}

#[test]
fn write_agent_config_file_creates_file_in_state_dir() {
    let state_dir = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    let config_path = write_agent_config_file(
        "test-agent-123",
        workspace.path(),
        None,
        &HashMap::new(),
        "done",
        None,
        state_dir.path(),
    )
    .unwrap();

    assert!(config_path.starts_with(state_dir.path()));
    assert!(config_path.exists());

    let expected_path =
        oj_core::agent_dir(state_dir.path(), "test-agent-123").join("agent-config.json");
    assert_eq!(config_path, expected_path);
}

#[test]
fn write_agent_config_file_no_hooks_injected() {
    let workspace = TempDir::new().unwrap();
    let (_state_dir, parsed) =
        config_for("test-no-hooks", workspace.path(), None, &HashMap::new(), "done", None);

    // No hooks of any kind in settings — priming is via start config
    let settings = &parsed["settings"];
    assert!(settings["hooks"].is_null());
}

#[test]
fn write_agent_config_file_merges_project_settings() {
    let workspace = TempDir::new().unwrap();
    let settings_dir = workspace.path().join(".claude");
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(settings_dir.join("settings.json"), r#"{"key": "value"}"#).unwrap();

    let (_state_dir, parsed) =
        config_for("test-merge", workspace.path(), None, &HashMap::new(), "done", None);

    assert_eq!(parsed["settings"]["key"], "value");
}

#[yare::parameterized(
    done = { "done", None, "allow" },
    fail = { "fail", None, "allow" },
    nudge = { "nudge", None, "gate" },
    gate = { "gate", None, "gate" },
    resume = { "resume", None, "gate" },
    escalate = { "escalate", None, "gate" },
    auto = { "auto", None, "auto" },
)]
fn stop_config_mode(on_idle: &str, message: Option<&str>, expected_mode: &str) {
    let workspace = TempDir::new().unwrap();
    let (_state_dir, parsed) =
        config_for("test-stop", workspace.path(), None, &HashMap::new(), on_idle, message);

    assert_eq!(parsed["stop"]["mode"], expected_mode);
    // gate mode has a prompt; allow/auto modes don't
    if expected_mode == "gate" {
        assert!(parsed["stop"]["prompt"].is_string());
    } else {
        assert!(parsed["stop"]["prompt"].is_null());
    }
}

#[test]
fn escalate_prompt_includes_ask_user_question() {
    let workspace = TempDir::new().unwrap();
    let (_state_dir, parsed) =
        config_for("test-escalate", workspace.path(), None, &HashMap::new(), "escalate", None);

    let prompt = parsed["stop"]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("AskUserQuestion"),
        "escalate prompt should include AskUserQuestion suffix: {}",
        prompt
    );
}

#[test]
fn write_agent_config_file_without_prime_no_start() {
    let workspace = TempDir::new().unwrap();
    let (_state_dir, parsed) =
        config_for("test-no-prime", workspace.path(), None, &HashMap::new(), "done", None);

    assert!(parsed["start"].is_null(), "start should be absent when no prime");
}

#[test]
fn write_agent_config_file_with_prime_has_start() {
    let workspace = TempDir::new().unwrap();
    let prime = PrimeDef::Script("echo hello".to_string());
    let (_state_dir, parsed) =
        config_for("test-prime", workspace.path(), Some(&prime), &HashMap::new(), "done", None);

    assert!(parsed["start"].is_object(), "start should be present when prime is set");
    let shell = parsed["start"]["shell"].as_array().unwrap();
    assert_eq!(shell.len(), 1);
    assert!(shell[0].as_str().unwrap().contains("echo hello"));
}

// --- build_start_config tests ---

#[test]
fn build_start_config_script_form() {
    let prime = PrimeDef::Script("echo hello\ngit status".to_string());
    let config = build_start_config(&prime, &HashMap::new());

    let shell = config["shell"].as_array().unwrap();
    assert_eq!(shell.len(), 1);
    let script = shell[0].as_str().unwrap();
    assert!(script.starts_with("set -euo pipefail\n"));
    assert!(script.contains("echo hello\ngit status"));
}

#[test]
fn build_start_config_commands_form() {
    let prime = PrimeDef::Commands(vec!["echo hello".to_string(), "git status".to_string()]);
    let config = build_start_config(&prime, &HashMap::new());

    let shell = config["shell"].as_array().unwrap();
    assert_eq!(shell.len(), 1);
    let script = shell[0].as_str().unwrap();
    assert!(script.starts_with("set -euo pipefail\n"));
    assert!(script.contains("echo hello\ngit status"));
}

#[test]
fn build_start_config_per_source_form() {
    let mut map = HashMap::new();
    map.insert(
        "startup".to_string(),
        PrimeDef::Commands(vec!["echo startup".to_string(), "git status".to_string()]),
    );
    map.insert("resume".to_string(), PrimeDef::Script("echo resume".to_string()));
    let prime = PrimeDef::PerSource(map);
    let config = build_start_config(&prime, &HashMap::new());

    assert!(config["event"].is_object(), "should have event map");
    let event = config["event"].as_object().unwrap();
    assert_eq!(event.len(), 2);

    // Check startup entry
    let startup = &event["startup"];
    let startup_shell = startup["shell"].as_array().unwrap();
    assert!(startup_shell[0].as_str().unwrap().contains("echo startup\ngit status"));

    // Check resume entry
    let resume = &event["resume"];
    let resume_shell = resume["shell"].as_array().unwrap();
    assert!(resume_shell[0].as_str().unwrap().contains("echo resume"));
}

#[test]
fn build_start_config_interpolates_variables() {
    let prime = PrimeDef::Script("echo ${name} in ${workspace}".to_string());
    let vars: HashMap<String, String> = [
        ("name".to_string(), "test-job".to_string()),
        ("workspace".to_string(), "/tmp/ws".to_string()),
    ]
    .into_iter()
    .collect();

    let config = build_start_config(&prime, &vars);
    let script = config["shell"][0].as_str().unwrap();
    assert!(script.contains("echo test-job in /tmp/ws"), "script: {}", script);
}
