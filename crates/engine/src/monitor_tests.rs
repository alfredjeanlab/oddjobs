// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for monitor module

use super::*;
use oj_core::{Pipeline, PipelineId, StepStatus, TimerId};
use oj_runbook::{parse_runbook, ActionConfig, AgentAction, AgentDef};
use std::collections::HashMap;
use std::time::Instant;

fn test_pipeline() -> Pipeline {
    Pipeline {
        id: "test-1".to_string(),
        name: "test-feature".to_string(),
        kind: "build".to_string(),
        step: "execute".to_string(),
        step_status: StepStatus::Running,
        runbook_hash: "testhash".to_string(),
        cwd: std::path::PathBuf::from("/tmp/test"),
        session_id: Some("sess-1".to_string()),
        workspace_id: None,
        workspace_path: Some("/tmp/test".into()),
        vars: HashMap::new(),
        created_at: Instant::now(),
        step_started_at: Instant::now(),
        error: None,
        step_history: Vec::new(),
        action_attempts: HashMap::new(),
        agent_signal: None,
        namespace: String::new(),
        cancelling: false,
        total_retries: 0,
        step_visits: HashMap::new(),
    }
}

fn test_agent_def() -> AgentDef {
    AgentDef {
        name: "worker".to_string(),
        run: "claude".to_string(),
        prompt: Some("Do the task.".to_string()),
        ..Default::default()
    }
}

#[test]
fn nudge_builds_send_effect() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Nudge);

    let result = build_action_effects(&pipeline, &agent, &config, "idle", &HashMap::new());
    assert!(matches!(result, Ok(ActionEffects::Nudge { .. })));
}

#[test]
fn done_returns_advance_pipeline() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Done);

    let result = build_action_effects(&pipeline, &agent, &config, "idle", &HashMap::new());
    assert!(matches!(result, Ok(ActionEffects::AdvancePipeline)));
}

#[test]
fn fail_returns_fail_pipeline() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Fail);

    let result = build_action_effects(&pipeline, &agent, &config, "error", &HashMap::new());
    assert!(matches!(result, Ok(ActionEffects::FailPipeline { .. })));
}

#[test]
fn recover_returns_recover_effects() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Recover);

    let result = build_action_effects(&pipeline, &agent, &config, "exit", &HashMap::new());
    assert!(matches!(result, Ok(ActionEffects::Recover { .. })));
}

#[test]
fn recover_with_message_replaces_prompt() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::with_message(AgentAction::Recover, "New prompt.");
    let input = [("prompt".to_string(), "Original".to_string())]
        .into_iter()
        .collect();

    let result = build_action_effects(&pipeline, &agent, &config, "exit", &input).unwrap();
    if let ActionEffects::Recover { input, .. } = result {
        assert_eq!(input.get("prompt"), Some(&"New prompt.".to_string()));
    } else {
        panic!("Expected Recover");
    }
}

#[test]
fn recover_with_append_appends_to_prompt() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::with_append(AgentAction::Recover, "Try again.");
    let input = [("prompt".to_string(), "Original".to_string())]
        .into_iter()
        .collect();

    let result = build_action_effects(&pipeline, &agent, &config, "exit", &input).unwrap();
    if let ActionEffects::Recover { input, .. } = result {
        let prompt = input.get("prompt").unwrap();
        assert!(prompt.contains("Original"));
        assert!(prompt.contains("Try again."));
    } else {
        panic!("Expected Recover");
    }
}

#[test]
fn escalate_returns_escalate_effects() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&pipeline, &agent, &config, "idle", &HashMap::new());
    assert!(matches!(result, Ok(ActionEffects::Escalate { .. })));
}

#[test]
fn escalate_step_waiting_has_no_reason() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result =
        build_action_effects(&pipeline, &agent, &config, "gate_failed", &HashMap::new()).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let step_waiting = effects.iter().find(|e| {
            matches!(
                e,
                oj_core::Effect::Emit {
                    event: oj_core::Event::StepWaiting { .. }
                }
            )
        });
        assert!(step_waiting.is_some(), "should emit StepWaiting");
        if let Some(oj_core::Effect::Emit {
            event: oj_core::Event::StepWaiting { reason, .. },
        }) = step_waiting
        {
            assert!(
                reason.is_none(),
                "escalate without gate should have no reason"
            );
        }
    } else {
        panic!("Expected Escalate");
    }
}

// Tests for get_agent_def

const RUNBOOK_WITH_AGENT: &str = r#"
[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "execute"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the task"
"#;

const RUNBOOK_WITHOUT_AGENT: &str = r#"
[pipeline.build]
input  = ["name"]

[[pipeline.build.step]]
name = "execute"
run = "echo hello"
"#;

#[test]
fn get_agent_def_finds_agent() {
    let runbook = parse_runbook(RUNBOOK_WITH_AGENT).unwrap();
    let pipeline = test_pipeline();

    let agent = get_agent_def(&runbook, &pipeline).unwrap();
    assert_eq!(agent.name, "worker");
}

#[test]
fn get_agent_def_fails_on_missing_pipeline() {
    let runbook = parse_runbook(RUNBOOK_WITH_AGENT).unwrap();
    let mut pipeline = test_pipeline();
    pipeline.kind = "nonexistent".to_string();

    let result = get_agent_def(&runbook, &pipeline);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("nonexistent"));
}

#[test]
fn get_agent_def_fails_on_missing_step() {
    let runbook = parse_runbook(RUNBOOK_WITH_AGENT).unwrap();
    let mut pipeline = test_pipeline();
    pipeline.step = "nonexistent".to_string();

    let result = get_agent_def(&runbook, &pipeline);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("nonexistent"));
}

#[test]
fn get_agent_def_fails_on_non_agent_step() {
    let runbook = parse_runbook(RUNBOOK_WITHOUT_AGENT).unwrap();
    let pipeline = test_pipeline();

    let result = get_agent_def(&runbook, &pipeline);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("not an agent step"));
}

// Test gate action

#[test]
fn gate_returns_gate_effects() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::WithOptions {
        action: AgentAction::Gate,
        message: None,
        append: false,
        run: Some("make test".to_string()),
        attempts: oj_runbook::Attempts::default(),
        cooldown: None,
    };

    let result = build_action_effects(&pipeline, &agent, &config, "exit", &HashMap::new());
    assert!(matches!(result, Ok(ActionEffects::Gate { .. })));
    if let Ok(ActionEffects::Gate { command, .. }) = result {
        assert_eq!(command, "make test");
    }
}

#[test]
fn gate_without_run_field_errors() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Gate);

    let result = build_action_effects(&pipeline, &agent, &config, "exit", &HashMap::new());
    assert!(result.is_err());
}

// Test nudge without session_id

#[test]
fn nudge_fails_without_session_id() {
    let mut pipeline = test_pipeline();
    pipeline.session_id = None;
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Nudge);

    let result = build_action_effects(&pipeline, &agent, &config, "idle", &HashMap::new());
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("no session"));
}

#[test]
fn escalate_cancels_exit_deferred_but_keeps_liveness() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&pipeline, &agent, &config, "idle", &HashMap::new()).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let cancelled_timer_ids: Vec<&str> = effects
            .iter()
            .filter_map(|e| {
                if let oj_core::Effect::CancelTimer { id } = e {
                    Some(id.as_str())
                } else {
                    None
                }
            })
            .collect();

        let expected_liveness = TimerId::liveness(&PipelineId::new(&pipeline.id));
        let expected_exit_deferred = TimerId::exit_deferred(&PipelineId::new(&pipeline.id));

        assert!(
            !cancelled_timer_ids.contains(&expected_liveness.as_str()),
            "should NOT cancel liveness timer (agent still running), got: {:?}",
            cancelled_timer_ids
        );
        assert!(
            cancelled_timer_ids.contains(&expected_exit_deferred.as_str()),
            "should cancel exit-deferred timer, got: {:?}",
            cancelled_timer_ids
        );
    } else {
        panic!("Expected Escalate");
    }
}

// =============================================================================
// Duration Parsing Tests
// =============================================================================

#[test]
fn parse_duration_seconds() {
    assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
    assert_eq!(parse_duration("1s").unwrap(), Duration::from_secs(1));
    assert_eq!(parse_duration("0s").unwrap(), Duration::from_secs(0));
    assert_eq!(parse_duration("30sec").unwrap(), Duration::from_secs(30));
    assert_eq!(parse_duration("30secs").unwrap(), Duration::from_secs(30));
    assert_eq!(parse_duration("30second").unwrap(), Duration::from_secs(30));
    assert_eq!(
        parse_duration("30seconds").unwrap(),
        Duration::from_secs(30)
    );
}

#[test]
fn parse_duration_minutes() {
    assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
    assert_eq!(parse_duration("1m").unwrap(), Duration::from_secs(60));
    assert_eq!(parse_duration("5min").unwrap(), Duration::from_secs(300));
    assert_eq!(parse_duration("5mins").unwrap(), Duration::from_secs(300));
    assert_eq!(parse_duration("5minute").unwrap(), Duration::from_secs(300));
    assert_eq!(
        parse_duration("5minutes").unwrap(),
        Duration::from_secs(300)
    );
}

#[test]
fn parse_duration_hours() {
    assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
    assert_eq!(parse_duration("1hr").unwrap(), Duration::from_secs(3600));
    assert_eq!(parse_duration("1hrs").unwrap(), Duration::from_secs(3600));
    assert_eq!(parse_duration("1hour").unwrap(), Duration::from_secs(3600));
    assert_eq!(parse_duration("1hours").unwrap(), Duration::from_secs(3600));
}

#[test]
fn parse_duration_days() {
    assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
    assert_eq!(parse_duration("1day").unwrap(), Duration::from_secs(86400));
    assert_eq!(parse_duration("1days").unwrap(), Duration::from_secs(86400));
}

#[test]
fn parse_duration_bare_number() {
    // Bare number defaults to seconds
    assert_eq!(parse_duration("30").unwrap(), Duration::from_secs(30));
}

#[test]
fn parse_duration_with_whitespace() {
    assert_eq!(parse_duration(" 30s ").unwrap(), Duration::from_secs(30));
    assert_eq!(parse_duration("30 s").unwrap(), Duration::from_secs(30));
}

#[test]
fn parse_duration_invalid_suffix() {
    let result = parse_duration("30x");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unknown duration suffix"));
}

#[test]
fn parse_duration_empty_string() {
    let result = parse_duration("");
    assert!(result.is_err());
}

#[test]
fn parse_duration_invalid_number() {
    let result = parse_duration("abcs");
    assert!(result.is_err());
}

#[test]
fn parse_duration_milliseconds() {
    assert_eq!(parse_duration("200ms").unwrap(), Duration::from_millis(200));
    assert_eq!(parse_duration("0ms").unwrap(), Duration::from_millis(0));
    assert_eq!(
        parse_duration("1500ms").unwrap(),
        Duration::from_millis(1500)
    );
    assert_eq!(
        parse_duration("100millis").unwrap(),
        Duration::from_millis(100)
    );
    assert_eq!(
        parse_duration("1millisecond").unwrap(),
        Duration::from_millis(1)
    );
    assert_eq!(
        parse_duration("50milliseconds").unwrap(),
        Duration::from_millis(50)
    );
}

// =============================================================================
// Agent Notification Tests
// =============================================================================

#[test]
fn agent_on_start_notify_renders_template() {
    let pipeline = test_pipeline();
    let mut agent = test_agent_def();
    agent.notify.on_start = Some("Agent ${agent} started for ${name}".to_string());

    let effect = build_agent_notify_effect(&pipeline, &agent, agent.notify.on_start.as_ref());
    assert!(effect.is_some());
    match effect.unwrap() {
        Effect::Notify { title, message } => {
            assert_eq!(title, "worker");
            assert_eq!(message, "Agent worker started for test-feature");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn agent_on_done_notify_renders_template() {
    let pipeline = test_pipeline();
    let mut agent = test_agent_def();
    agent.notify.on_done = Some("Agent ${agent} completed".to_string());

    let effect = build_agent_notify_effect(&pipeline, &agent, agent.notify.on_done.as_ref());
    match effect.unwrap() {
        Effect::Notify { title, message } => {
            assert_eq!(title, "worker");
            assert_eq!(message, "Agent worker completed");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn agent_on_fail_notify_includes_error() {
    let mut pipeline = test_pipeline();
    pipeline.error = Some("task failed".to_string());
    let mut agent = test_agent_def();
    agent.notify.on_fail = Some("Agent ${agent} failed: ${error}".to_string());

    let effect = build_agent_notify_effect(&pipeline, &agent, agent.notify.on_fail.as_ref());
    match effect.unwrap() {
        Effect::Notify { title, message } => {
            assert_eq!(title, "worker");
            assert_eq!(message, "Agent worker failed: task failed");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn agent_notify_none_when_no_template() {
    let pipeline = test_pipeline();
    let agent = test_agent_def();
    let effect = build_agent_notify_effect(&pipeline, &agent, None);
    assert!(effect.is_none());
}

#[test]
fn agent_notify_interpolates_pipeline_vars() {
    let mut pipeline = test_pipeline();
    pipeline.vars.insert("env".to_string(), "prod".to_string());
    let mut agent = test_agent_def();
    agent.notify.on_start = Some("Deploying ${var.env}".to_string());

    let effect = build_agent_notify_effect(&pipeline, &agent, agent.notify.on_start.as_ref());
    match effect.unwrap() {
        Effect::Notify { message, .. } => {
            assert_eq!(message, "Deploying prod");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn agent_notify_includes_step_variable() {
    let pipeline = test_pipeline();
    let mut agent = test_agent_def();
    agent.notify.on_start = Some("Step: ${step}".to_string());

    let effect = build_agent_notify_effect(&pipeline, &agent, agent.notify.on_start.as_ref());
    match effect.unwrap() {
        Effect::Notify { message, .. } => {
            assert_eq!(message, "Step: execute");
        }
        _ => panic!("expected Notify effect"),
    }
}

// =============================================================================
// Cooldown Timer ID Tests
// =============================================================================
