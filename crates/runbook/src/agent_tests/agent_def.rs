// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::super::*;

#[test]
fn agent_build_command_with_prompt_field() {
    // When prompt is configured via field, run command has no ${prompt}
    let agent = AgentDef {
        name: "planner".to_string(),
        run: "claude".to_string(),
        prompt: Some("Do something".to_string()),
        prompt_file: None,
        env: HashMap::new(),
        cwd: None,
        prime: None,
        on_idle: ActionConfig::default(),
        on_dead: default_on_dead(),
        on_prompt: default_on_prompt(),
        on_error: default_on_error(),
        on_stop: None,
        max_concurrency: None,
        notify: Default::default(),
    };

    let vars: HashMap<String, String> = HashMap::new();

    // build_command just interpolates the run template; session-id and prompt are added by spawn.rs
    assert_eq!(agent.build_command(&vars), "claude");
}

#[test]
fn agent_build_command_with_inline_prompt() {
    // When prompt is inline, run command contains ${prompt}
    let agent = AgentDef {
        name: "planner".to_string(),
        run: "claude \"${prompt}\"".to_string(),
        prompt: None,
        prompt_file: None,
        env: HashMap::new(),
        cwd: None,
        prime: None,
        on_idle: ActionConfig::default(),
        on_dead: default_on_dead(),
        on_prompt: default_on_prompt(),
        on_error: default_on_error(),
        on_stop: None,
        max_concurrency: None,
        notify: Default::default(),
    };

    let vars: HashMap<String, String> = [("prompt".to_string(), "Add login".to_string())]
        .into_iter()
        .collect();

    assert_eq!(agent.build_command(&vars), "claude \"Add login\"");
}

#[test]
fn agent_build_command_print_mode() {
    let agent = AgentDef {
        name: "planner".to_string(),
        run: "claude -p".to_string(),
        prompt: Some("Plan the task".to_string()),
        prompt_file: None,
        env: HashMap::new(),
        cwd: None,
        prime: None,
        on_idle: ActionConfig::default(),
        on_dead: default_on_dead(),
        on_prompt: default_on_prompt(),
        on_error: default_on_error(),
        on_stop: None,
        max_concurrency: None,
        notify: Default::default(),
    };

    let vars: HashMap<String, String> = HashMap::new();

    // build_command just interpolates the run template; session-id and prompt are added by spawn.rs
    assert_eq!(agent.build_command(&vars), "claude -p");
}

#[test]
fn agent_build_env() {
    let agent = AgentDef {
        name: "executor".to_string(),
        run: "claude".to_string(),
        prompt: Some("Execute the plan".to_string()),
        prompt_file: None,
        env: [
            ("OJ_STEP".to_string(), "execute".to_string()),
            ("OJ_NAME".to_string(), "${name}".to_string()),
        ]
        .into_iter()
        .collect(),
        cwd: None,
        prime: None,
        on_idle: ActionConfig::default(),
        on_dead: default_on_dead(),
        on_prompt: default_on_prompt(),
        on_error: default_on_error(),
        on_stop: None,
        max_concurrency: None,
        notify: Default::default(),
    };

    let vars: HashMap<String, String> = [
        ("job_id".to_string(), "pipe-1".to_string()),
        ("name".to_string(), "feature".to_string()),
    ]
    .into_iter()
    .collect();

    let env = agent.build_env(&vars);
    assert!(env.contains(&("OJ_STEP".to_string(), "execute".to_string())));
    assert!(env.contains(&("OJ_NAME".to_string(), "feature".to_string())));
}

#[test]
fn agent_get_prompt_from_field() {
    let agent = AgentDef {
        name: "worker".to_string(),
        run: "claude".to_string(),
        prompt: Some("Do ${task} for ${name}".to_string()),
        prompt_file: None,
        env: HashMap::new(),
        cwd: None,
        prime: None,
        on_idle: ActionConfig::default(),
        on_dead: default_on_dead(),
        on_prompt: default_on_prompt(),
        on_error: default_on_error(),
        on_stop: None,
        max_concurrency: None,
        notify: Default::default(),
    };

    let vars: HashMap<String, String> = [
        ("task".to_string(), "coding".to_string()),
        ("name".to_string(), "feature-1".to_string()),
    ]
    .into_iter()
    .collect();

    let prompt = agent.get_prompt(&vars).unwrap();
    assert_eq!(prompt, "Do coding for feature-1");
}

#[test]
fn agent_get_prompt_empty_when_unset() {
    let agent = AgentDef {
        name: "worker".to_string(),
        run: "claude".to_string(),
        prompt: None,
        prompt_file: None,
        env: HashMap::new(),
        cwd: None,
        prime: None,
        on_idle: ActionConfig::default(),
        on_dead: default_on_dead(),
        on_prompt: default_on_prompt(),
        on_error: default_on_error(),
        on_stop: None,
        max_concurrency: None,
        notify: Default::default(),
    };

    let vars = HashMap::new();
    let prompt = agent.get_prompt(&vars).unwrap();
    assert_eq!(prompt, "");
}

#[test]
fn agent_get_prompt_from_file() {
    use std::io::Write;
    use tempfile::NamedTempFile;

    let mut file = NamedTempFile::new().unwrap();
    writeln!(file, "Do ${{task}} for ${{name}}").unwrap();

    let agent = AgentDef {
        name: "worker".to_string(),
        run: "claude".to_string(),
        prompt: None,
        prompt_file: Some(file.path().to_path_buf()),
        env: HashMap::new(),
        cwd: None,
        prime: None,
        on_idle: ActionConfig::default(),
        on_dead: default_on_dead(),
        on_prompt: default_on_prompt(),
        on_error: default_on_error(),
        on_stop: None,
        max_concurrency: None,
        notify: Default::default(),
    };

    let vars: HashMap<String, String> = [
        ("task".to_string(), "coding".to_string()),
        ("name".to_string(), "feature-1".to_string()),
    ]
    .into_iter()
    .collect();

    let prompt = agent.get_prompt(&vars).unwrap();
    assert!(prompt.contains("Do coding for feature-1"));
}

#[test]
fn agent_get_prompt_file_not_found() {
    let agent = AgentDef {
        name: "worker".to_string(),
        run: "claude".to_string(),
        prompt: None,
        prompt_file: Some(PathBuf::from("/nonexistent/path/to/prompt.md")),
        env: HashMap::new(),
        cwd: None,
        prime: None,
        on_idle: ActionConfig::default(),
        on_dead: default_on_dead(),
        on_prompt: default_on_prompt(),
        on_error: default_on_error(),
        on_stop: None,
        max_concurrency: None,
        notify: Default::default(),
    };

    let vars = HashMap::new();
    assert!(agent.get_prompt(&vars).is_err());
}
