// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

// =============================================================================
// PrimeDef Tests
// =============================================================================

#[test]
fn prime_deserialize_string_form() {
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        prime: PrimeDef,
    }

    let toml = r#"
        prime = "echo hello\ngit status"
    "#;
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert!(matches!(config.prime, PrimeDef::Script(_)));
    if let PrimeDef::Script(s) = &config.prime {
        assert!(s.contains("echo hello"));
    }
}

#[test]
fn prime_deserialize_array_form() {
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        prime: PrimeDef,
    }

    let toml = r#"
        prime = ["echo hello", "git status"]
    "#;
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert!(matches!(config.prime, PrimeDef::Commands(_)));
    if let PrimeDef::Commands(cmds) = &config.prime {
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], "echo hello");
        assert_eq!(cmds[1], "git status");
    }
}

#[test]
fn prime_render_script_interpolates() {
    let prime = PrimeDef::Script("echo ${name} in ${workspace}".to_string());
    let vars: HashMap<String, String> = [
        ("name".to_string(), "test-feature".to_string()),
        ("workspace".to_string(), "/tmp/ws".to_string()),
    ]
    .into_iter()
    .collect();

    let result = prime.render(&vars);
    assert_eq!(result, "echo test-feature in /tmp/ws");
}

#[test]
fn prime_render_commands_interpolates() {
    let prime = PrimeDef::Commands(vec![
        "echo ${name}".to_string(),
        "git -C ${workspace} status".to_string(),
    ]);
    let vars: HashMap<String, String> = [
        ("name".to_string(), "test-feature".to_string()),
        ("workspace".to_string(), "/tmp/ws".to_string()),
    ]
    .into_iter()
    .collect();

    let result = prime.render(&vars);
    assert_eq!(result, "echo test-feature\ngit -C /tmp/ws status");
}

#[test]
fn prime_optional_defaults_to_none() {
    let toml = r#"
        name = "worker"
        run = "claude"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    assert!(agent.prime.is_none());
}

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
        notify: Default::default(),
    };

    let vars: HashMap<String, String> = [
        ("pipeline_id".to_string(), "pipe-1".to_string()),
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
        notify: Default::default(),
    };

    let vars = HashMap::new();
    assert!(agent.get_prompt(&vars).is_err());
}

// =============================================================================
// Action Configuration Tests
// =============================================================================

#[test]
fn parses_simple_action() {
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default)]
        on_idle: ActionConfig,
        #[serde(default = "default_on_dead")]
        on_dead: ActionConfig,
    }

    let toml = r#"
        on_idle = "nudge"
        on_dead = "escalate"
    "#;
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.on_idle.action(), &AgentAction::Nudge);
    assert_eq!(config.on_dead.action(), &AgentAction::Escalate);
}

#[test]
fn parses_action_with_message() {
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default)]
        on_idle: ActionConfig,
    }

    let toml = r#"
        on_idle = { action = "nudge", message = "Keep going" }
    "#;
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.on_idle.action(), &AgentAction::Nudge);
    assert_eq!(config.on_idle.message(), Some("Keep going"));
    assert!(!config.on_idle.append());
}

#[test]
fn parses_action_with_append() {
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default = "default_on_dead")]
        on_dead: ActionConfig,
    }

    let toml = r#"
        on_dead = { action = "recover", message = "Previous attempt exited.", append = true }
    "#;
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.on_dead.action(), &AgentAction::Recover);
    assert_eq!(config.on_dead.message(), Some("Previous attempt exited."));
    assert!(config.on_dead.append());
}

#[test]
fn parses_per_error_actions() {
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default = "default_on_error")]
        on_error: ErrorActionConfig,
    }

    let toml = r#"
        [[on_error]]
        match = "no_internet"
        action = "recover"
        message = "Network restored"

        [[on_error]]
        action = "escalate"
    "#;
    let config: TestConfig = toml::from_str(toml).unwrap();

    // Match specific error type
    let action = config.on_error.action_for(Some(&ErrorType::NoInternet));
    assert_eq!(action.action(), &AgentAction::Recover);
    assert_eq!(action.message(), Some("Network restored"));

    // Fall through to catch-all
    let action = config.on_error.action_for(Some(&ErrorType::Unauthorized));
    assert_eq!(action.action(), &AgentAction::Escalate);
}

#[test]
fn error_action_config_simple() {
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default = "default_on_error")]
        on_error: ErrorActionConfig,
    }

    let toml = r#"
        on_error = "escalate"
    "#;
    let config: TestConfig = toml::from_str(toml).unwrap();

    let action = config.on_error.action_for(Some(&ErrorType::NoInternet));
    assert_eq!(action.action(), &AgentAction::Escalate);
}

#[test]
fn error_action_config_default_when_no_match() {
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default = "default_on_error")]
        on_error: ErrorActionConfig,
    }

    // Only matches rate_limited, no catch-all
    let toml = r#"
        [[on_error]]
        match = "rate_limited"
        action = "recover"
    "#;
    let config: TestConfig = toml::from_str(toml).unwrap();

    // Should default to escalate when no match
    let action = config.on_error.action_for(Some(&ErrorType::NoInternet));
    assert_eq!(action.action(), &AgentAction::Escalate);
}

#[test]
fn action_config_defaults() {
    // Defaults: on_idle = "nudge", on_dead = "escalate", on_error = "escalate"
    let default_idle = ActionConfig::default();
    assert_eq!(default_idle.action(), &AgentAction::Nudge);

    let default_exit = default_on_dead();
    assert_eq!(default_exit.action(), &AgentAction::Escalate);

    let default_error = default_on_error();
    let action = default_error.action_for(Some(&ErrorType::Unauthorized));
    assert_eq!(action.action(), &AgentAction::Escalate);
}

#[test]
fn parses_full_agent_with_actions() {
    let toml = r#"
        name = "worker"
        run = "claude -p"
        prompt = "Do the task."
        on_idle = { action = "nudge", message = "Keep going" }
        on_dead = "escalate"
        on_error = "escalate"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    assert_eq!(agent.on_idle.action(), &AgentAction::Nudge);
    assert_eq!(agent.on_idle.message(), Some("Keep going"));
    assert_eq!(agent.on_dead.action(), &AgentAction::Escalate);
}

// =============================================================================
// Attempts Parsing Tests
// =============================================================================

#[test]
fn attempts_default_is_one() {
    assert_eq!(Attempts::default(), Attempts::Finite(1));
}

#[test]
fn attempts_finite_parses() {
    let toml = r#"
        on_idle = { action = "nudge", attempts = 3 }
    "#;
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default)]
        on_idle: ActionConfig,
    }
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.on_idle.attempts(), Attempts::Finite(3));
}

#[test]
fn attempts_forever_parses() {
    let toml = r#"
        on_idle = { action = "nudge", attempts = "forever" }
    "#;
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default)]
        on_idle: ActionConfig,
    }
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.on_idle.attempts(), Attempts::Forever);
}

#[test]
fn attempts_zero_fails() {
    let toml = r#"
        on_idle = { action = "nudge", attempts = 0 }
    "#;
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct TestConfig {
        #[serde(default)]
        on_idle: ActionConfig,
    }
    let result: Result<TestConfig, _> = toml::from_str(toml);
    assert!(result.is_err(), "expected attempts = 0 to fail parsing");
}

#[test]
fn attempts_invalid_string_fails() {
    let toml = r#"
        on_idle = { action = "nudge", attempts = "infinite" }
    "#;
    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct TestConfig {
        #[serde(default)]
        on_idle: ActionConfig,
    }
    let result: Result<TestConfig, _> = toml::from_str(toml);
    assert!(result.is_err());
}

#[test]
fn attempts_missing_defaults_to_one() {
    let toml = r#"
        on_idle = { action = "nudge" }
    "#;
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default)]
        on_idle: ActionConfig,
    }
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.on_idle.attempts(), Attempts::Finite(1));
}

#[test]
fn attempts_simple_action_returns_default() {
    let toml = r#"
        on_idle = "nudge"
    "#;
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default)]
        on_idle: ActionConfig,
    }
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.on_idle.attempts(), Attempts::Finite(1));
}

#[test]
fn attempts_is_exhausted() {
    let finite = Attempts::Finite(3);
    assert!(!finite.is_exhausted(0));
    assert!(!finite.is_exhausted(1));
    assert!(!finite.is_exhausted(2));
    assert!(finite.is_exhausted(3));
    assert!(finite.is_exhausted(100));

    let forever = Attempts::Forever;
    assert!(!forever.is_exhausted(0));
    assert!(!forever.is_exhausted(1000));
    assert!(!forever.is_exhausted(u32::MAX));
}

#[test]
fn cooldown_parses() {
    let toml = r#"
        on_idle = { action = "nudge", cooldown = "30s" }
    "#;
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default)]
        on_idle: ActionConfig,
    }
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.on_idle.cooldown(), Some("30s"));
}

#[test]
fn cooldown_simple_action_returns_none() {
    let config = ActionConfig::Simple(AgentAction::Nudge);
    assert_eq!(config.cooldown(), None);
}

#[test]
fn parses_action_with_attempts_and_cooldown() {
    let toml = r#"
        on_idle = { action = "nudge", message = "Keep going", attempts = 5, cooldown = "1m" }
    "#;
    #[derive(Debug, Deserialize)]
    struct TestConfig {
        #[serde(default)]
        on_idle: ActionConfig,
    }
    let config: TestConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.on_idle.action(), &AgentAction::Nudge);
    assert_eq!(config.on_idle.message(), Some("Keep going"));
    assert_eq!(config.on_idle.attempts(), Attempts::Finite(5));
    assert_eq!(config.on_idle.cooldown(), Some("1m"));
}

// =============================================================================
// Notify Config Tests
// =============================================================================

#[test]
fn parses_agent_with_notify() {
    let toml = r#"
        name = "worker"
        run = "claude"
        prompt = "Do the task."
        on_idle = "nudge"
        on_dead = "escalate"

        [notify]
        on_start = "Agent started: ${name}"
        on_done  = "Agent completed"
        on_fail  = "Agent failed"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    assert_eq!(
        agent.notify.on_start.as_deref(),
        Some("Agent started: ${name}")
    );
    assert_eq!(agent.notify.on_done.as_deref(), Some("Agent completed"));
    assert_eq!(agent.notify.on_fail.as_deref(), Some("Agent failed"));
}

#[test]
fn agent_notify_defaults_to_empty() {
    let toml = r#"
        name = "worker"
        run = "claude"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    assert!(agent.notify.on_start.is_none());
    assert!(agent.notify.on_done.is_none());
    assert!(agent.notify.on_fail.is_none());
}

#[test]
fn agent_notify_partial() {
    let toml = r#"
        name = "worker"
        run = "claude"

        [notify]
        on_fail = "Worker failed!"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    assert!(agent.notify.on_start.is_none());
    assert!(agent.notify.on_done.is_none());
    assert_eq!(agent.notify.on_fail.as_deref(), Some("Worker failed!"));
}

// =============================================================================
// on_prompt Tests
// =============================================================================

#[test]
fn on_prompt_defaults_to_escalate() {
    let agent = AgentDef::default();
    assert_eq!(agent.on_prompt.action(), &AgentAction::Escalate);
}

#[test]
fn on_prompt_parses_simple() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_prompt = "done"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    assert_eq!(agent.on_prompt.action(), &AgentAction::Done);
}

#[test]
fn on_prompt_parses_with_options() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_prompt = { action = "gate", run = "check-permissions.sh" }
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    assert_eq!(agent.on_prompt.action(), &AgentAction::Gate);
    assert_eq!(agent.on_prompt.run(), Some("check-permissions.sh"));
}

#[test]
fn on_prompt_missing_defaults_to_escalate() {
    let toml = r#"
        name = "worker"
        run = "claude"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    assert_eq!(agent.on_prompt.action(), &AgentAction::Escalate);
}

#[test]
fn on_prompt_trigger_validation() {
    // Valid actions for OnPrompt
    assert!(AgentAction::Escalate.is_valid_for_trigger(ActionTrigger::OnPrompt));
    assert!(AgentAction::Done.is_valid_for_trigger(ActionTrigger::OnPrompt));
    assert!(AgentAction::Fail.is_valid_for_trigger(ActionTrigger::OnPrompt));
    assert!(AgentAction::Gate.is_valid_for_trigger(ActionTrigger::OnPrompt));

    // Invalid actions for OnPrompt
    assert!(!AgentAction::Nudge.is_valid_for_trigger(ActionTrigger::OnPrompt));
    assert!(!AgentAction::Recover.is_valid_for_trigger(ActionTrigger::OnPrompt));
}
