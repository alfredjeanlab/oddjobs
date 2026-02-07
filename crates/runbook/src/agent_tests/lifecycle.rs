// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::super::*;

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
    assert!(!AgentAction::Resume.is_valid_for_trigger(ActionTrigger::OnPrompt));
}

// =============================================================================
// Session Config Tests
// =============================================================================

#[test]
fn session_config_parses_from_toml() {
    let toml = r#"
        name = "worker"
        run = "claude"

        [session.tmux]
        color = "cyan"
        title = "mayor"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    let tmux = agent.session.get("tmux").unwrap();
    assert_eq!(tmux.color.as_deref(), Some("cyan"));
    assert_eq!(tmux.title.as_deref(), Some("mayor"));
    assert!(tmux.status.is_none());
}

#[test]
fn session_config_parses_with_status() {
    let toml = r#"
        name = "worker"
        run = "claude"

        [session.tmux]
        color = "green"

        [session.tmux.status]
        left = "myproject merge/check"
        right = "custom-id"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    let tmux = agent.session.get("tmux").unwrap();
    assert_eq!(tmux.color.as_deref(), Some("green"));
    let status = tmux.status.as_ref().unwrap();
    assert_eq!(status.left.as_deref(), Some("myproject merge/check"));
    assert_eq!(status.right.as_deref(), Some("custom-id"));
}

#[test]
fn session_config_empty_when_absent() {
    let toml = r#"
        name = "worker"
        run = "claude"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    assert!(agent.session.is_empty());
}

#[test]
fn session_config_unknown_provider_parses() {
    // Unknown providers should parse without error (ignored at adapter level)
    let toml = r#"
        name = "worker"
        run = "claude"

        [session.zellij]
        color = "red"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    // "zellij" is treated as TmuxSessionConfig since the map value type is fixed
    assert!(agent.session.contains_key("zellij"));
}

#[test]
fn session_config_partial_fields() {
    let toml = r#"
        name = "worker"
        run = "claude"

        [session.tmux]
        title = "my-worker"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    let tmux = agent.session.get("tmux").unwrap();
    assert!(tmux.color.is_none());
    assert_eq!(tmux.title.as_deref(), Some("my-worker"));
    assert!(tmux.status.is_none());
}

#[test]
fn session_config_serialization_roundtrip() {
    let config = TmuxSessionConfig {
        color: Some("blue".to_string()),
        title: Some("test".to_string()),
        status: Some(SessionStatusConfig {
            left: Some("left text".to_string()),
            right: Some("right text".to_string()),
        }),
    };

    let json = serde_json::to_string(&config).unwrap();
    let parsed: TmuxSessionConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.color.as_deref(), Some("blue"));
    assert_eq!(parsed.title.as_deref(), Some("test"));
    let status = parsed.status.unwrap();
    assert_eq!(status.left.as_deref(), Some("left text"));
    assert_eq!(status.right.as_deref(), Some("right text"));
}

#[test]
fn session_config_interpolate() {
    let config = TmuxSessionConfig {
        color: Some("blue".to_string()),
        title: Some("Bug: ${var.bug.id}".to_string()),
        status: Some(SessionStatusConfig {
            left: Some("${var.bug.id}: ${var.bug.title}".to_string()),
            right: Some("${workspace.branch}".to_string()),
        }),
    };

    let vars: HashMap<String, String> = [
        ("var.bug.id".to_string(), "BUG-123".to_string()),
        ("var.bug.title".to_string(), "Fix login".to_string()),
        ("workspace.branch".to_string(), "fix/bug-123".to_string()),
    ]
    .into_iter()
    .collect();

    let interpolated = config.interpolate(&vars);

    // Color is not interpolated
    assert_eq!(interpolated.color.as_deref(), Some("blue"));
    // Title is interpolated
    assert_eq!(interpolated.title.as_deref(), Some("Bug: BUG-123"));
    // Status left/right are interpolated
    let status = interpolated.status.unwrap();
    assert_eq!(status.left.as_deref(), Some("BUG-123: Fix login"));
    assert_eq!(status.right.as_deref(), Some("fix/bug-123"));
}

#[test]
fn session_config_interpolate_missing_vars_preserved() {
    let config = TmuxSessionConfig {
        color: None,
        title: Some("${unknown.var}".to_string()),
        status: None,
    };

    let vars = HashMap::new();
    let interpolated = config.interpolate(&vars);

    // Missing variables are preserved as-is
    assert_eq!(interpolated.title.as_deref(), Some("${unknown.var}"));
}

// =============================================================================
// on_stop Tests
// =============================================================================

#[test]
fn on_stop_simple_signal() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_stop = "signal"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    let config = agent.on_stop.unwrap();
    assert_eq!(config.action(), &StopAction::Signal);
}

#[test]
fn on_stop_simple_idle() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_stop = "idle"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    let config = agent.on_stop.unwrap();
    assert_eq!(config.action(), &StopAction::Idle);
}

#[test]
fn on_stop_simple_escalate() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_stop = "escalate"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    let config = agent.on_stop.unwrap();
    assert_eq!(config.action(), &StopAction::Escalate);
}

#[test]
fn on_stop_object_form() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_stop = { action = "idle" }
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    let config = agent.on_stop.unwrap();
    assert_eq!(config.action(), &StopAction::Idle);
}

#[test]
fn on_stop_default_is_none() {
    let toml = r#"
        name = "worker"
        run = "claude"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    assert!(agent.on_stop.is_none());
}

#[test]
fn on_stop_invalid_value_rejected() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_stop = "nudge"
    "#;
    let result: Result<AgentDef, _> = toml::from_str(toml);
    assert!(
        result.is_err(),
        "on_stop = 'nudge' should be rejected as invalid"
    );
}

#[test]
fn on_stop_invalid_object_value_rejected() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_stop = { action = "done" }
    "#;
    let result: Result<AgentDef, _> = toml::from_str(toml);
    assert!(
        result.is_err(),
        "on_stop = {{ action = 'done' }} should be rejected as invalid"
    );
}

// =============================================================================
// on_stop = "ask" Tests
// =============================================================================

#[test]
fn on_stop_simple_ask_parses() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_stop = "ask"
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    let config = agent.on_stop.unwrap();
    assert_eq!(config.action(), &StopAction::Ask);
    assert!(config.message().is_none());
}

#[test]
fn on_stop_ask_with_message() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_stop = { action = "ask", message = "What should I do next?" }
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    let config = agent.on_stop.unwrap();
    assert_eq!(config.action(), &StopAction::Ask);
    assert_eq!(config.message(), Some("What should I do next?"));
}

#[test]
fn on_stop_ask_object_without_message() {
    let toml = r#"
        name = "worker"
        run = "claude"
        on_stop = { action = "ask" }
    "#;
    let agent: AgentDef = toml::from_str(toml).unwrap();
    let config = agent.on_stop.unwrap();
    assert_eq!(config.action(), &StopAction::Ask);
    assert!(config.message().is_none());
}

// =============================================================================
// on_idle = "ask" Trigger Validation Tests
// =============================================================================

#[test]
fn ask_is_valid_for_on_idle() {
    assert!(AgentAction::Ask.is_valid_for_trigger(ActionTrigger::OnIdle));
}

#[test]
fn ask_is_invalid_for_on_dead() {
    assert!(!AgentAction::Ask.is_valid_for_trigger(ActionTrigger::OnDead));
}

#[test]
fn ask_is_invalid_for_on_error() {
    assert!(!AgentAction::Ask.is_valid_for_trigger(ActionTrigger::OnError));
}

#[test]
fn ask_is_invalid_for_on_prompt() {
    assert!(!AgentAction::Ask.is_valid_for_trigger(ActionTrigger::OnPrompt));
}
