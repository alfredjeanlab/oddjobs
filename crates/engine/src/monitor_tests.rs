// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for monitor module

use super::*;
use oj_core::{Job, JobId, TimerId};
use oj_runbook::{parse_runbook, ActionConfig, AgentAction, AgentDef};

fn test_job() -> Job {
    Job::builder()
        .name("test-feature")
        .session_id("sess-1")
        .workspace_path("/tmp/test")
        .build()
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
fn nudge_builds_send_effect() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Nudge);

    let result = build_action_effects(&test_ctx(&agent, &config, "idle"), &job);
    assert!(matches!(result, Ok(ActionEffects::Nudge { .. })));
}

#[test]
fn done_returns_advance_job() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Done);

    let result = build_action_effects(&test_ctx(&agent, &config, "idle"), &job);
    assert!(matches!(result, Ok(ActionEffects::AdvanceJob)));
}

#[test]
fn fail_returns_fail_job() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Fail);

    let result = build_action_effects(&test_ctx(&agent, &config, "error"), &job);
    assert!(matches!(result, Ok(ActionEffects::FailJob { .. })));
}

#[test]
fn resume_returns_resume_effects() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Resume);

    let result = build_action_effects(&test_ctx(&agent, &config, "exit"), &job);
    assert!(matches!(result, Ok(ActionEffects::Resume { .. })));
}

#[test]
fn resume_with_message_replaces_prompt() {
    let mut job = test_job();
    job.vars
        .insert("prompt".to_string(), "Original".to_string());
    let agent = test_agent_def();
    let config = ActionConfig::with_message(AgentAction::Resume, "New prompt.");

    let result = build_action_effects(&test_ctx(&agent, &config, "exit"), &job).unwrap();
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
        panic!("Expected Resume");
    }
}

#[test]
fn resume_with_append_sets_resume_message() {
    let mut job = test_job();
    job.vars
        .insert("prompt".to_string(), "Original".to_string());
    let agent = test_agent_def();
    let config = ActionConfig::with_append(AgentAction::Resume, "Try again.");

    let result = build_action_effects(&test_ctx(&agent, &config, "exit"), &job).unwrap();
    if let ActionEffects::Resume {
        input,
        resume_session_id,
        ..
    } = result
    {
        // In append+resume mode, message goes to resume_message, not prompt
        assert_eq!(input.get("resume_message"), Some(&"Try again.".to_string()));
        // Original prompt should not be modified
        assert_eq!(input.get("prompt"), Some(&"Original".to_string()));
        // resume_session_id is None here because test_job() has no step_history,
        // but the code path for append mode does set use_resume=true internally
        assert!(
            resume_session_id.is_none(),
            "no prior session in test fixture"
        );
    } else {
        panic!("Expected Resume");
    }
}

#[test]
fn resume_without_message_uses_resume_session() {
    let mut job = test_job();
    // Add a step history record with an agent_id to simulate previous run
    job.step_history.push(oj_core::StepRecord {
        name: "execute".to_string(),
        started_at_ms: 0,
        finished_at_ms: None,
        outcome: oj_core::StepOutcome::Running,
        agent_id: Some("prev-session-uuid".to_string()),
        agent_name: Some("worker".to_string()),
    });
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Resume);

    let result = build_action_effects(&test_ctx(&agent, &config, "exit"), &job).unwrap();
    if let ActionEffects::Resume {
        resume_session_id, ..
    } = result
    {
        assert_eq!(resume_session_id, Some("prev-session-uuid".to_string()));
    } else {
        panic!("Expected Resume");
    }
}

#[test]
fn resume_with_no_prior_session_falls_back() {
    let job = test_job(); // no step_history
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Resume);

    let result = build_action_effects(&test_ctx(&agent, &config, "exit"), &job).unwrap();
    if let ActionEffects::Resume {
        resume_session_id, ..
    } = result
    {
        assert!(
            resume_session_id.is_none(),
            "should be None when no step history"
        );
    } else {
        panic!("Expected Resume");
    }
}

#[test]
fn escalate_returns_escalate_effects() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&test_ctx(&agent, &config, "idle"), &job);
    assert!(matches!(result, Ok(ActionEffects::Escalate { .. })));
}

#[test]
fn escalate_emits_decision_created() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&test_ctx(&agent, &config, "gate_failed"), &job).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let decision_created = effects.iter().find(|e| {
            matches!(
                e,
                oj_core::Effect::Emit {
                    event: oj_core::Event::DecisionCreated { .. }
                }
            )
        });
        assert!(decision_created.is_some(), "should emit DecisionCreated");

        // Verify the decision has the correct source for gate_failed trigger
        // (gate_failed ends with _exhausted pattern, so it maps to Idle as fallback)
        if let Some(oj_core::Effect::Emit {
            event:
                oj_core::Event::DecisionCreated {
                    source, options, ..
                },
        }) = decision_created
        {
            // Escalation from gate_failed trigger should create a decision with options
            assert!(!options.is_empty(), "should have options");
            // The source depends on how the trigger is parsed
            assert!(
                matches!(source, oj_core::DecisionSource::Idle),
                "gate_failed trigger maps to Idle source, got {:?}",
                source
            );
        }
    } else {
        panic!("Expected Escalate");
    }
}

// Tests for get_agent_def

const RUNBOOK_WITH_AGENT: &str = r#"
[job.build]
input  = ["name"]

[[job.build.step]]
name = "execute"
run = { agent = "worker" }

[agent.worker]
run = 'claude'
prompt = "Do the task"
"#;

const RUNBOOK_WITHOUT_AGENT: &str = r#"
[job.build]
input  = ["name"]

[[job.build.step]]
name = "execute"
run = "echo hello"
"#;

#[test]
fn get_agent_def_finds_agent() {
    let runbook = parse_runbook(RUNBOOK_WITH_AGENT).unwrap();
    let job = test_job();

    let agent = get_agent_def(&runbook, &job).unwrap();
    assert_eq!(agent.name, "worker");
}

#[test]
fn get_agent_def_fails_on_missing_job() {
    let runbook = parse_runbook(RUNBOOK_WITH_AGENT).unwrap();
    let mut job = test_job();
    job.kind = "nonexistent".to_string();

    let result = get_agent_def(&runbook, &job);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("nonexistent"));
}

#[test]
fn get_agent_def_fails_on_missing_step() {
    let runbook = parse_runbook(RUNBOOK_WITH_AGENT).unwrap();
    let mut job = test_job();
    job.step = "nonexistent".to_string();

    let result = get_agent_def(&runbook, &job);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("nonexistent"));
}

#[test]
fn get_agent_def_fails_on_non_agent_step() {
    let runbook = parse_runbook(RUNBOOK_WITHOUT_AGENT).unwrap();
    let job = test_job();

    let result = get_agent_def(&runbook, &job);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("not an agent step"));
}

// Test gate action

#[test]
fn gate_returns_gate_effects() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::WithOptions {
        action: AgentAction::Gate,
        message: None,
        append: false,
        run: Some("make test".to_string()),
        attempts: oj_runbook::Attempts::default(),
        cooldown: None,
    };

    let result = build_action_effects(&test_ctx(&agent, &config, "exit"), &job);
    assert!(matches!(result, Ok(ActionEffects::Gate { .. })));
    if let Ok(ActionEffects::Gate { command, .. }) = result {
        assert_eq!(command, "make test");
    }
}

#[test]
fn gate_without_run_field_errors() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Gate);

    let result = build_action_effects(&test_ctx(&agent, &config, "exit"), &job);
    assert!(result.is_err());
}

// Test nudge without session_id

#[test]
fn nudge_fails_without_session_id() {
    let mut job = test_job();
    job.session_id = None;
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Nudge);

    let result = build_action_effects(&test_ctx(&agent, &config, "idle"), &job);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("no session"));
}

#[test]
fn escalate_cancels_exit_deferred_but_keeps_liveness() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&test_ctx(&agent, &config, "idle"), &job).unwrap();
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

        let expected_liveness = TimerId::liveness(&JobId::new(&job.id));
        let expected_exit_deferred = TimerId::exit_deferred(&JobId::new(&job.id));

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
    let job = test_job();
    let mut agent = test_agent_def();
    agent.notify.on_start = Some("Agent ${agent} started for ${name}".to_string());

    let effect = build_agent_notify_effect(&job, &agent, agent.notify.on_start.as_ref());
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
    let job = test_job();
    let mut agent = test_agent_def();
    agent.notify.on_done = Some("Agent ${agent} completed".to_string());

    let effect = build_agent_notify_effect(&job, &agent, agent.notify.on_done.as_ref());
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
    let mut job = test_job();
    job.error = Some("task failed".to_string());
    let mut agent = test_agent_def();
    agent.notify.on_fail = Some("Agent ${agent} failed: ${error}".to_string());

    let effect = build_agent_notify_effect(&job, &agent, agent.notify.on_fail.as_ref());
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
    let job = test_job();
    let agent = test_agent_def();
    let effect = build_agent_notify_effect(&job, &agent, None);
    assert!(effect.is_none());
}

#[test]
fn agent_notify_interpolates_job_vars() {
    let mut job = test_job();
    job.vars.insert("env".to_string(), "prod".to_string());
    let mut agent = test_agent_def();
    agent.notify.on_start = Some("Deploying ${var.env}".to_string());

    let effect = build_agent_notify_effect(&job, &agent, agent.notify.on_start.as_ref());
    match effect.unwrap() {
        Effect::Notify { message, .. } => {
            assert_eq!(message, "Deploying prod");
        }
        _ => panic!("expected Notify effect"),
    }
}

#[test]
fn agent_notify_includes_step_variable() {
    let job = test_job();
    let mut agent = test_agent_def();
    agent.notify.on_start = Some("Step: ${step}".to_string());

    let effect = build_agent_notify_effect(&job, &agent, agent.notify.on_start.as_ref());
    match effect.unwrap() {
        Effect::Notify { message, .. } => {
            assert_eq!(message, "Step: execute");
        }
        _ => panic!("expected Notify effect"),
    }
}

// =============================================================================
// MonitorState Conversion Tests
// =============================================================================

#[test]
fn monitor_state_from_working() {
    let state = MonitorState::from_agent_state(&oj_core::AgentState::Working);
    assert!(matches!(state, MonitorState::Working));
}

#[test]
fn monitor_state_from_waiting_for_input() {
    let state = MonitorState::from_agent_state(&oj_core::AgentState::WaitingForInput);
    assert!(matches!(state, MonitorState::WaitingForInput));
}

#[test]
fn monitor_state_from_failed_unauthorized() {
    let state = MonitorState::from_agent_state(&oj_core::AgentState::Failed(
        oj_core::AgentError::Unauthorized,
    ));
    match state {
        MonitorState::Failed {
            message,
            error_type,
        } => {
            assert!(message.contains("nauthorized"));
            assert_eq!(error_type, Some(oj_runbook::ErrorType::Unauthorized));
        }
        _ => panic!("expected Failed"),
    }
}

#[test]
fn monitor_state_from_failed_out_of_credits() {
    let state = MonitorState::from_agent_state(&oj_core::AgentState::Failed(
        oj_core::AgentError::OutOfCredits,
    ));
    match state {
        MonitorState::Failed { error_type, .. } => {
            assert_eq!(error_type, Some(oj_runbook::ErrorType::OutOfCredits));
        }
        _ => panic!("expected Failed"),
    }
}

#[test]
fn monitor_state_from_failed_no_internet() {
    let state = MonitorState::from_agent_state(&oj_core::AgentState::Failed(
        oj_core::AgentError::NoInternet,
    ));
    match state {
        MonitorState::Failed { error_type, .. } => {
            assert_eq!(error_type, Some(oj_runbook::ErrorType::NoInternet));
        }
        _ => panic!("expected Failed"),
    }
}

#[test]
fn monitor_state_from_failed_rate_limited() {
    let state = MonitorState::from_agent_state(&oj_core::AgentState::Failed(
        oj_core::AgentError::RateLimited,
    ));
    match state {
        MonitorState::Failed { error_type, .. } => {
            assert_eq!(error_type, Some(oj_runbook::ErrorType::RateLimited));
        }
        _ => panic!("expected Failed"),
    }
}

#[test]
fn monitor_state_from_failed_other() {
    let state = MonitorState::from_agent_state(&oj_core::AgentState::Failed(
        oj_core::AgentError::Other("custom error".to_string()),
    ));
    match state {
        MonitorState::Failed {
            message,
            error_type,
        } => {
            assert!(message.contains("custom error"));
            assert_eq!(error_type, None);
        }
        _ => panic!("expected Failed"),
    }
}

#[test]
fn monitor_state_from_exited() {
    let state = MonitorState::from_agent_state(&oj_core::AgentState::Exited { exit_code: Some(0) });
    assert!(matches!(state, MonitorState::Exited));
}

#[test]
fn monitor_state_from_session_gone() {
    let state = MonitorState::from_agent_state(&oj_core::AgentState::SessionGone);
    assert!(matches!(state, MonitorState::Gone));
}

// =============================================================================
// Agent Failure to Error Type Mapping
// =============================================================================

#[test]
fn agent_failure_unauthorized_maps_to_error_type() {
    assert_eq!(
        agent_failure_to_error_type(&oj_core::AgentError::Unauthorized),
        Some(oj_runbook::ErrorType::Unauthorized)
    );
}

#[test]
fn agent_failure_out_of_credits_maps_to_error_type() {
    assert_eq!(
        agent_failure_to_error_type(&oj_core::AgentError::OutOfCredits),
        Some(oj_runbook::ErrorType::OutOfCredits)
    );
}

#[test]
fn agent_failure_no_internet_maps_to_error_type() {
    assert_eq!(
        agent_failure_to_error_type(&oj_core::AgentError::NoInternet),
        Some(oj_runbook::ErrorType::NoInternet)
    );
}

#[test]
fn agent_failure_rate_limited_maps_to_error_type() {
    assert_eq!(
        agent_failure_to_error_type(&oj_core::AgentError::RateLimited),
        Some(oj_runbook::ErrorType::RateLimited)
    );
}

#[test]
fn agent_failure_other_maps_to_none() {
    assert_eq!(
        agent_failure_to_error_type(&oj_core::AgentError::Other("anything".to_string())),
        None
    );
}

// =============================================================================
// Escalation Trigger Mapping Tests
// =============================================================================

#[test]
fn escalate_idle_trigger_emits_idle_source() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&test_ctx(&agent, &config, "idle"), &job).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let source = effects.iter().find_map(|e| match e {
            oj_core::Effect::Emit {
                event: oj_core::Event::DecisionCreated { source, .. },
            } => Some(source.clone()),
            _ => None,
        });
        assert_eq!(source, Some(oj_core::DecisionSource::Idle));
    } else {
        panic!("expected Escalate");
    }
}

#[test]
fn escalate_exit_trigger_emits_error_source() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&test_ctx(&agent, &config, "exit"), &job).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let source = effects.iter().find_map(|e| match e {
            oj_core::Effect::Emit {
                event: oj_core::Event::DecisionCreated { source, .. },
            } => Some(source.clone()),
            _ => None,
        });
        assert_eq!(source, Some(oj_core::DecisionSource::Error));
    } else {
        panic!("expected Escalate");
    }
}

#[test]
fn escalate_error_trigger_emits_error_source() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&test_ctx(&agent, &config, "error"), &job).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let source = effects.iter().find_map(|e| match e {
            oj_core::Effect::Emit {
                event: oj_core::Event::DecisionCreated { source, .. },
            } => Some(source.clone()),
            _ => None,
        });
        assert_eq!(source, Some(oj_core::DecisionSource::Error));
    } else {
        panic!("expected Escalate");
    }
}

#[test]
fn escalate_prompt_trigger_emits_approval_source() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&test_ctx(&agent, &config, "prompt"), &job).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let source = effects.iter().find_map(|e| match e {
            oj_core::Effect::Emit {
                event: oj_core::Event::DecisionCreated { source, .. },
            } => Some(source.clone()),
            _ => None,
        });
        assert_eq!(source, Some(oj_core::DecisionSource::Approval));
    } else {
        panic!("expected Escalate");
    }
}

#[test]
fn escalate_prompt_question_trigger_emits_question_source() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&test_ctx(&agent, &config, "prompt:question"), &job).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let source = effects.iter().find_map(|e| match e {
            oj_core::Effect::Emit {
                event: oj_core::Event::DecisionCreated { source, .. },
            } => Some(source.clone()),
            _ => None,
        });
        assert_eq!(source, Some(oj_core::DecisionSource::Question));
    } else {
        panic!("expected Escalate");
    }
}

#[test]
fn escalate_idle_exhausted_trigger_emits_idle_source() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&test_ctx(&agent, &config, "idle:exhausted"), &job).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let source = effects.iter().find_map(|e| match e {
            oj_core::Effect::Emit {
                event: oj_core::Event::DecisionCreated { source, .. },
            } => Some(source.clone()),
            _ => None,
        });
        assert_eq!(source, Some(oj_core::DecisionSource::Idle));
    } else {
        panic!("expected Escalate");
    }
}

#[test]
fn escalate_error_exhausted_trigger_emits_error_source() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects(&test_ctx(&agent, &config, "error:exhausted"), &job).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let source = effects.iter().find_map(|e| match e {
            oj_core::Effect::Emit {
                event: oj_core::Event::DecisionCreated { source, .. },
            } => Some(source.clone()),
            _ => None,
        });
        assert_eq!(source, Some(oj_core::DecisionSource::Error));
    } else {
        panic!("expected Escalate");
    }
}

#[test]
fn escalate_unknown_trigger_falls_back_to_idle() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result =
        build_action_effects(&test_ctx(&agent, &config, "some_unknown_trigger"), &job).unwrap();
    if let ActionEffects::Escalate { effects } = result {
        let source = effects.iter().find_map(|e| match e {
            oj_core::Effect::Emit {
                event: oj_core::Event::DecisionCreated { source, .. },
            } => Some(source.clone()),
            _ => None,
        });
        assert_eq!(source, Some(oj_core::DecisionSource::Idle));
    } else {
        panic!("expected Escalate");
    }
}

// =============================================================================
// Nudge Message Content Tests
// =============================================================================

#[test]
fn nudge_uses_default_message() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Nudge);

    let result = build_action_effects(&test_ctx(&agent, &config, "idle"), &job).unwrap();
    if let ActionEffects::Nudge { effects } = result {
        match &effects[0] {
            Effect::SendToSession { input, .. } => {
                assert_eq!(input, "Please continue with the task.\n");
            }
            _ => panic!("expected SendToSession"),
        }
    } else {
        panic!("expected Nudge");
    }
}

#[test]
fn nudge_uses_custom_message() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::with_message(AgentAction::Nudge, "Keep going!");

    let result = build_action_effects(&test_ctx(&agent, &config, "idle"), &job).unwrap();
    if let ActionEffects::Nudge { effects } = result {
        match &effects[0] {
            Effect::SendToSession { input, .. } => {
                assert_eq!(input, "Keep going!\n");
            }
            _ => panic!("expected SendToSession"),
        }
    } else {
        panic!("expected Nudge");
    }
}

// =============================================================================
// Fail Action Tests
// =============================================================================

#[test]
fn fail_uses_trigger_as_error_message() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Fail);

    let result = build_action_effects(&test_ctx(&agent, &config, "on_error"), &job).unwrap();
    if let ActionEffects::FailJob { error } = result {
        assert_eq!(error, "on_error");
    } else {
        panic!("expected FailJob");
    }
}

// =============================================================================
// Standalone Agent Run Action Building Tests
// =============================================================================

fn test_agent_run() -> oj_core::AgentRun {
    oj_core::AgentRun::builder().build()
}

#[test]
fn agent_run_nudge_builds_send_effect() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Nudge);

    let result = build_action_effects_for_agent_run(&test_ctx(&agent, &config, "idle"), &ar);
    assert!(matches!(result, Ok(ActionEffects::Nudge { .. })));
}

#[test]
fn agent_run_nudge_without_session_fails() {
    let mut ar = test_agent_run();
    ar.session_id = None;
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Nudge);

    let result = build_action_effects_for_agent_run(&test_ctx(&agent, &config, "idle"), &ar);
    assert!(result.is_err());
}

#[test]
fn agent_run_nudge_custom_message() {
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
fn agent_run_done_returns_complete() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Done);

    let result = build_action_effects_for_agent_run(&test_ctx(&agent, &config, "idle"), &ar);
    assert!(matches!(result, Ok(ActionEffects::CompleteAgentRun)));
}

#[test]
fn agent_run_fail_returns_fail() {
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
fn agent_run_resume_returns_resume_effects() {
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
fn agent_run_resume_with_replace_message() {
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
fn agent_run_resume_with_append_message() {
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
fn agent_run_escalate_returns_escalate_effects() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    let result = build_action_effects_for_agent_run(&test_ctx(&agent, &config, "idle"), &ar);
    assert!(matches!(result, Ok(ActionEffects::EscalateAgentRun { .. })));
}

#[test]
fn agent_run_escalate_emits_decision_and_status_change() {
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
fn agent_run_escalate_trigger_mapping() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);

    // Test "exit" trigger maps to Error source
    let result =
        build_action_effects_for_agent_run(&test_ctx(&agent, &config, "exit"), &ar).unwrap();
    if let ActionEffects::EscalateAgentRun { effects } = result {
        let source = effects.iter().find_map(|e| match e {
            oj_core::Effect::Emit {
                event: oj_core::Event::DecisionCreated { source, .. },
            } => Some(source.clone()),
            _ => None,
        });
        assert_eq!(source, Some(oj_core::DecisionSource::Error));
    }
}

#[test]
fn agent_run_gate_returns_gate_effects() {
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
fn agent_run_gate_without_run_errors() {
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
fn agent_run_notify_renders_template() {
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
fn agent_run_notify_includes_error() {
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
fn agent_run_notify_includes_vars() {
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
fn agent_run_notify_none_when_no_template() {
    let ar = test_agent_run();
    let agent = test_agent_def();
    let effect = build_agent_run_notify_effect(&ar, &agent, None);
    assert!(effect.is_none());
}

// =============================================================================
// Cooldown Timer ID Tests
// =============================================================================
