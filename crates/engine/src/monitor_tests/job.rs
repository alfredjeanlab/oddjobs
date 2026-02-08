// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for job-context action effects building

use crate::monitor::*;
use oj_core::{Effect, Job, JobId, TimerId};
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
fn simple_action_variant_checks() {
    let job = test_job();
    let agent = test_agent_def();

    let c = ActionConfig::simple(AgentAction::Nudge);
    assert!(matches!(
        build_action_effects(&test_ctx(&agent, &c, "idle"), &job),
        Ok(ActionEffects::Nudge { .. })
    ));

    let c = ActionConfig::simple(AgentAction::Done);
    assert!(matches!(
        build_action_effects(&test_ctx(&agent, &c, "idle"), &job),
        Ok(ActionEffects::AdvanceJob)
    ));

    let c = ActionConfig::simple(AgentAction::Fail);
    assert!(matches!(
        build_action_effects(&test_ctx(&agent, &c, "error"), &job),
        Ok(ActionEffects::FailJob { .. })
    ));

    let c = ActionConfig::simple(AgentAction::Resume);
    assert!(matches!(
        build_action_effects(&test_ctx(&agent, &c, "exit"), &job),
        Ok(ActionEffects::Resume { .. })
    ));
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

#[yare::parameterized(
    secs_30s        = { "30s",              Duration::from_secs(30) },
    secs_1s         = { "1s",               Duration::from_secs(1) },
    secs_0s         = { "0s",               Duration::from_secs(0) },
    secs_30sec      = { "30sec",            Duration::from_secs(30) },
    secs_30secs     = { "30secs",           Duration::from_secs(30) },
    secs_30second   = { "30second",         Duration::from_secs(30) },
    secs_30seconds  = { "30seconds",        Duration::from_secs(30) },
    mins_5m         = { "5m",               Duration::from_secs(300) },
    mins_1m         = { "1m",               Duration::from_secs(60) },
    mins_5min       = { "5min",             Duration::from_secs(300) },
    mins_5mins      = { "5mins",            Duration::from_secs(300) },
    mins_5minute    = { "5minute",          Duration::from_secs(300) },
    mins_5minutes   = { "5minutes",         Duration::from_secs(300) },
    hours_1h        = { "1h",               Duration::from_secs(3600) },
    hours_2h        = { "2h",               Duration::from_secs(7200) },
    hours_1hr       = { "1hr",              Duration::from_secs(3600) },
    hours_1hrs      = { "1hrs",             Duration::from_secs(3600) },
    hours_1hour     = { "1hour",            Duration::from_secs(3600) },
    hours_1hours    = { "1hours",           Duration::from_secs(3600) },
    days_1d         = { "1d",               Duration::from_secs(86400) },
    days_1day       = { "1day",             Duration::from_secs(86400) },
    days_1days      = { "1days",            Duration::from_secs(86400) },
    bare_number     = { "30",               Duration::from_secs(30) },
    ws_leading      = { " 30s ",            Duration::from_secs(30) },
    ws_middle       = { "30 s",             Duration::from_secs(30) },
    ms_200          = { "200ms",            Duration::from_millis(200) },
    ms_0            = { "0ms",              Duration::from_millis(0) },
    ms_1500         = { "1500ms",           Duration::from_millis(1500) },
    millis_100      = { "100millis",        Duration::from_millis(100) },
    millisecond_1   = { "1millisecond",     Duration::from_millis(1) },
    milliseconds_50 = { "50milliseconds",   Duration::from_millis(50) },
)]
fn parse_duration_valid(input: &str, expected: Duration) {
    assert_eq!(parse_duration(input).unwrap(), expected);
}

#[yare::parameterized(
    invalid_suffix = { "30x" },
    empty_string   = { "" },
    invalid_number = { "abcs" },
)]
fn parse_duration_invalid(input: &str) {
    assert!(parse_duration(input).is_err());
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
fn monitor_state_simple_conversions() {
    use oj_core::AgentState;
    assert!(matches!(
        MonitorState::from_agent_state(&AgentState::Working),
        MonitorState::Working
    ));
    assert!(matches!(
        MonitorState::from_agent_state(&AgentState::WaitingForInput),
        MonitorState::WaitingForInput
    ));
    assert!(matches!(
        MonitorState::from_agent_state(&AgentState::Exited { exit_code: Some(0) }),
        MonitorState::Exited { exit_code: Some(0) }
    ));
    assert!(matches!(
        MonitorState::from_agent_state(&AgentState::SessionGone),
        MonitorState::Gone
    ));
}

#[yare::parameterized(
    unauthorized   = { oj_core::AgentError::Unauthorized,   Some(oj_runbook::ErrorType::Unauthorized) },
    out_of_credits = { oj_core::AgentError::OutOfCredits,   Some(oj_runbook::ErrorType::OutOfCredits) },
    no_internet    = { oj_core::AgentError::NoInternet,     Some(oj_runbook::ErrorType::NoInternet) },
    rate_limited   = { oj_core::AgentError::RateLimited,    Some(oj_runbook::ErrorType::RateLimited) },
)]
fn monitor_state_from_failed_has_error_type(
    error: oj_core::AgentError,
    expected: Option<oj_runbook::ErrorType>,
) {
    let state = MonitorState::from_agent_state(&oj_core::AgentState::Failed(error));
    match state {
        MonitorState::Failed { error_type, .. } => assert_eq!(error_type, expected),
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

// =============================================================================
// Agent Failure to Error Type Mapping
// =============================================================================

#[yare::parameterized(
    unauthorized   = { oj_core::AgentError::Unauthorized,                  Some(oj_runbook::ErrorType::Unauthorized) },
    out_of_credits = { oj_core::AgentError::OutOfCredits,                  Some(oj_runbook::ErrorType::OutOfCredits) },
    no_internet    = { oj_core::AgentError::NoInternet,                    Some(oj_runbook::ErrorType::NoInternet) },
    rate_limited   = { oj_core::AgentError::RateLimited,                   Some(oj_runbook::ErrorType::RateLimited) },
    other          = { oj_core::AgentError::Other("anything".to_string()), None },
)]
fn agent_failure_maps_to_error_type(
    error: oj_core::AgentError,
    expected: Option<oj_runbook::ErrorType>,
) {
    assert_eq!(agent_failure_to_error_type(&error), expected);
}

// =============================================================================
// Escalation Trigger Mapping Tests
// =============================================================================

fn extract_escalation_source(effects: &ActionEffects) -> oj_core::DecisionSource {
    if let ActionEffects::Escalate { effects } = effects {
        effects
            .iter()
            .find_map(|e| match e {
                oj_core::Effect::Emit {
                    event: oj_core::Event::DecisionCreated { source, .. },
                } => Some(source.clone()),
                _ => None,
            })
            .expect("should have DecisionCreated")
    } else {
        panic!("expected Escalate");
    }
}

#[yare::parameterized(
    idle              = { "idle",               oj_core::DecisionSource::Idle },
    exit              = { "exit",               oj_core::DecisionSource::Dead },
    error             = { "error",              oj_core::DecisionSource::Error },
    prompt            = { "prompt",             oj_core::DecisionSource::Approval },
    prompt_question   = { "prompt:question",    oj_core::DecisionSource::Question },
    idle_exhausted    = { "idle:exhausted",     oj_core::DecisionSource::Idle },
    error_exhausted   = { "error:exhausted",    oj_core::DecisionSource::Error },
    unknown_trigger   = { "some_unknown_trigger", oj_core::DecisionSource::Idle },
)]
fn escalate_trigger_maps_to_source(trigger: &str, expected: oj_core::DecisionSource) {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::simple(AgentAction::Escalate);
    let result = build_action_effects(&test_ctx(&agent, &config, trigger), &job).unwrap();
    assert_eq!(extract_escalation_source(&result), expected);
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
// Ask Action Tests (Job Context)
// =============================================================================

#[test]
fn ask_generates_nudge_with_ask_user_question_message() {
    let job = test_job();
    let agent = test_agent_def();
    let config = ActionConfig::with_message(AgentAction::Ask, "What should I work on next?");

    let result = build_action_effects(&test_ctx(&agent, &config, "idle"), &job).unwrap();
    if let ActionEffects::Nudge { effects } = result {
        match &effects[0] {
            Effect::SendToSession { input, .. } => {
                assert!(
                    input.contains("AskUserQuestion"),
                    "should mention AskUserQuestion: {}",
                    input
                );
                assert!(
                    input.contains("What should I work on next?"),
                    "should contain the topic: {}",
                    input
                );
            }
            _ => panic!("expected SendToSession"),
        }
    } else {
        panic!("expected Nudge, got {:?}", result);
    }
}

#[test]
fn ask_without_session_fails() {
    let mut job = test_job();
    job.session_id = None;
    let agent = test_agent_def();
    let config = ActionConfig::with_message(AgentAction::Ask, "What next?");

    let result = build_action_effects(&test_ctx(&agent, &config, "idle"), &job);
    assert!(result.is_err());
}
