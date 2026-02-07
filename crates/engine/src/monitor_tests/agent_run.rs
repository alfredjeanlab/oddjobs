// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for standalone agent run action effects building

use crate::monitor::*;
use oj_core::Effect;
use oj_runbook::{ActionConfig, AgentAction, AgentDef};

fn test_agent_run() -> oj_core::AgentRun {
    oj_core::AgentRun::builder().build()
}

fn test_agent_def() -> AgentDef {
    AgentDef {
        name: "worker".to_string(),
        run: "claude".to_string(),
        prompt: Some("Do the task.".to_string()),
        ..Default::default()
    }
}

fn test_ctx<'a>(
    agent_def: &'a AgentDef,
    action_config: &'a ActionConfig,
    trigger: &'a str,
) -> ActionContext<'a> {
    ActionContext {
        agent_def,
        action_config,
        trigger,
        chain_pos: 0,
        question_data: None,
        assistant_context: None,
    }
}

#[test]
fn simple_action_variant_checks() {
    let ar = test_agent_run();
    let agent = test_agent_def();

    let c = ActionConfig::simple(AgentAction::Nudge);
    assert!(matches!(
        build_action_effects_for_agent_run(&test_ctx(&agent, &c, "idle"), &ar),
        Ok(ActionEffects::Nudge { .. })
    ));

    let c = ActionConfig::simple(AgentAction::Done);
    assert!(matches!(
        build_action_effects_for_agent_run(&test_ctx(&agent, &c, "idle"), &ar),
        Ok(ActionEffects::CompleteAgentRun)
    ));

    let c = ActionConfig::simple(AgentAction::Escalate);
    assert!(matches!(
        build_action_effects_for_agent_run(&test_ctx(&agent, &c, "idle"), &ar),
        Ok(ActionEffects::EscalateAgentRun { .. })
    ));
}

#[test]
fn nudge_without_session_fails() {
    let mut ar = test_agent_run();
    ar.session_id = None;
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Nudge);

    let result = build_action_effects_for_agent_run(&test_ctx(&agent, &config, "idle"), &ar);
    assert!(result.is_err());
}

#[test]
fn nudge_custom_message() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::with_message(AgentAction::Nudge, "Custom nudge");

    let result =
        build_action_effects_for_agent_run(&test_ctx(&agent, &config, "idle"), &ar).unwrap();
    if let ActionEffects::Nudge { effects } = result {
        match &effects[0] {
            Effect::SendToSession { input, .. } => {
                assert_eq!(input, "Custom nudge\n");
            }
            _ => panic!("expected SendToSession"),
        }
    } else {
        panic!("expected Nudge");
    }
}

#[test]
fn fail_returns_fail() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Fail);

    let result =
        build_action_effects_for_agent_run(&test_ctx(&agent, &config, "error"), &ar).unwrap();
    if let ActionEffects::FailAgentRun { error } = result {
        assert_eq!(error, "error");
    } else {
        panic!("expected FailAgentRun");
    }
}

#[test]
fn resume_returns_resume_effects() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Resume);

    let result =
        build_action_effects_for_agent_run(&test_ctx(&agent, &config, "exit"), &ar).unwrap();
    if let ActionEffects::Resume {
        kill_session,
        agent_name,
        resume_session_id,
        ..
    } = result
    {
        assert_eq!(kill_session.as_deref(), Some("sess-1"));
        assert_eq!(agent_name, "worker");
        // Without message, resume_session_id comes from agent_run.agent_id
        assert_eq!(resume_session_id, Some("agent-uuid-1".to_string()));
    } else {
        panic!("expected Resume");
    }
}

#[test]
fn resume_with_replace_message() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::with_message(AgentAction::Resume, "New prompt.");

    let result =
        build_action_effects_for_agent_run(&test_ctx(&agent, &config, "exit"), &ar).unwrap();
    if let ActionEffects::Resume {
        input,
        resume_session_id,
        ..
    } = result
    {
        assert_eq!(input.get("prompt"), Some(&"New prompt.".to_string()));
        assert!(
            resume_session_id.is_none(),
            "replace mode should not use --resume"
        );
    } else {
        panic!("expected Resume");
    }
}

#[test]
fn resume_with_append_message() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::with_append(AgentAction::Resume, "Try again.");

    let result =
        build_action_effects_for_agent_run(&test_ctx(&agent, &config, "exit"), &ar).unwrap();
    if let ActionEffects::Resume {
        input,
        resume_session_id,
        ..
    } = result
    {
        assert_eq!(input.get("resume_message"), Some(&"Try again.".to_string()));
        assert_eq!(resume_session_id, Some("agent-uuid-1".to_string()));
    } else {
        panic!("expected Resume");
    }
}

#[test]
fn escalate_emits_decision_and_status_change() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result =
        build_action_effects_for_agent_run(&test_ctx(&agent, &config, "idle"), &ar).unwrap();
    if let ActionEffects::EscalateAgentRun { effects } = result {
        // Should emit DecisionCreated
        let has_decision = effects.iter().any(|e| {
            matches!(
                e,
                oj_core::Effect::Emit {
                    event: oj_core::Event::DecisionCreated { .. }
                }
            )
        });
        assert!(has_decision, "should emit DecisionCreated");

        // Should emit AgentRunStatusChanged to Escalated
        let has_status_change = effects.iter().any(|e| {
            matches!(
                e,
                oj_core::Effect::Emit {
                    event: oj_core::Event::AgentRunStatusChanged {
                        status: oj_core::AgentRunStatus::Escalated,
                        ..
                    }
                }
            )
        });
        assert!(has_status_change, "should emit AgentRunStatusChanged");

        // Should have a Notify effect
        let has_notify = effects
            .iter()
            .any(|e| matches!(e, oj_core::Effect::Notify { .. }));
        assert!(has_notify, "should have desktop notification");

        // Should cancel exit-deferred timer
        let has_cancel = effects
            .iter()
            .any(|e| matches!(e, oj_core::Effect::CancelTimer { .. }));
        assert!(has_cancel, "should cancel exit-deferred timer");
    } else {
        panic!("expected EscalateAgentRun");
    }
}

#[test]
fn escalate_trigger_mapping() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    // Test "exit" trigger maps to Dead source
    let result =
        build_action_effects_for_agent_run(&test_ctx(&agent, &config, "exit"), &ar).unwrap();
    if let ActionEffects::EscalateAgentRun { effects } = result {
        let source = effects.iter().find_map(|e| match e {
            oj_core::Effect::Emit {
                event: oj_core::Event::DecisionCreated { source, .. },
            } => Some(source.clone()),
            _ => None,
        });
        assert_eq!(source, Some(oj_core::DecisionSource::Dead));
    }
}

#[test]
fn gate_returns_gate_effects() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::WithOptions {
        action: AgentAction::Gate,
        message: None,
        append: false,
        run: Some("make test".to_string()),
        attempts: oj_runbook::Attempts::default(),
        cooldown: None,
    };

    let result =
        build_action_effects_for_agent_run(&test_ctx(&agent, &config, "exit"), &ar).unwrap();
    if let ActionEffects::Gate { command } = result {
        assert_eq!(command, "make test");
    } else {
        panic!("expected Gate");
    }
}

#[test]
fn gate_without_run_errors() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Gate);

    let result = build_action_effects_for_agent_run(&test_ctx(&agent, &config, "exit"), &ar);
    assert!(result.is_err());
}

// =============================================================================
// Standalone Agent Run Notification Tests
// =============================================================================

#[test]
fn notify_renders_template() {
    let ar = test_agent_run();
    let mut agent = test_agent_def();
    agent.notify.on_start = Some("Agent ${agent} running ${name}".to_string());

    let effect = build_agent_run_notify_effect(&ar, &agent, agent.notify.on_start.as_ref());
    assert!(effect.is_some());
    match effect.unwrap() {
        Effect::Notify { title, message } => {
            assert_eq!(title, "worker");
            assert_eq!(message, "Agent worker running agent_cmd");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn notify_includes_error() {
    let mut ar = test_agent_run();
    ar.error = Some("something failed".to_string());
    let mut agent = test_agent_def();
    agent.notify.on_fail = Some("Error: ${error}".to_string());

    let effect = build_agent_run_notify_effect(&ar, &agent, agent.notify.on_fail.as_ref());
    match effect.unwrap() {
        Effect::Notify { message, .. } => {
            assert_eq!(message, "Error: something failed");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn notify_includes_vars() {
    let mut ar = test_agent_run();
    ar.vars.insert("env".to_string(), "staging".to_string());
    let mut agent = test_agent_def();
    agent.notify.on_done = Some("Done in ${var.env}".to_string());

    let effect = build_agent_run_notify_effect(&ar, &agent, agent.notify.on_done.as_ref());
    match effect.unwrap() {
        Effect::Notify { message, .. } => {
            assert_eq!(message, "Done in staging");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn notify_none_when_no_template() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let effect = build_agent_run_notify_effect(&ar, &agent, None);
    assert!(effect.is_none());
}

// =============================================================================
// Ask Action Tests (Agent Run Context)
// =============================================================================

#[test]
fn ask_generates_nudge() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::with_message(AgentAction::Ask, "What should I do next?");

    let result =
        build_action_effects_for_agent_run(&test_ctx(&agent, &config, "idle"), &ar).unwrap();
    if let ActionEffects::Nudge { effects } = result {
        match &effects[0] {
            Effect::SendToSession { input, .. } => {
                assert!(input.contains("AskUserQuestion"));
                assert!(input.contains("What should I do next?"));
            }
            _ => panic!("expected SendToSession"),
        }
    } else {
        panic!("expected Nudge");
    }
}

#[test]
fn ask_without_session_fails() {
    let mut ar = test_agent_run();
    ar.session_id = None;
    let agent = test_agent_def();
    let config = ActionConfig::with_message(AgentAction::Ask, "What next?");

    let result = build_action_effects_for_agent_run(&test_ctx(&agent, &config, "idle"), &ar);
    assert!(result.is_err());
}
