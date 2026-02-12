// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for monitor action effects and guard invariants.

// ── Job-specific action tests ─────────────────────────────────────────

mod job {
    use crate::engine::monitor::*;
    use oj_core::Job;
    use oj_runbook::{parse_runbook, ActionConfig, AgentAction, AgentDef};
    use std::time::Duration;

    fn test_job() -> Job {
        Job::builder().name("test-feature").workspace_path("/tmp/test").build()
    }
    fn test_agent_def() -> AgentDef {
        AgentDef {
            name: "worker".into(),
            run: "claude".into(),
            prompt: Some("Do the task.".into()),
            ..Default::default()
        }
    }
    fn test_ctx<'a>(
        def: &'a AgentDef,
        cfg: &'a ActionConfig,
        trigger: &'a str,
    ) -> ActionContext<'a> {
        ActionContext {
            agent_def: def,
            action_config: cfg,
            trigger,
            chain_pos: 0,
            questions: None,
            last_message: None,
        }
    }

    #[test]
    fn resume_with_message_replaces_prompt() {
        let mut job = test_job();
        job.vars.insert("prompt".into(), "Original".into());
        let agent = test_agent_def();
        let config = ActionConfig::with_message(AgentAction::Resume, "New prompt.");
        let result = build_action_effects_for(&test_ctx(&agent, &config, "exit"), &job).unwrap();
        if let ActionEffects::Resume { input, resume, .. } = result {
            assert_eq!(input.get("prompt"), Some(&"New prompt.".to_string()));
            assert!(!resume, "replace mode should not use --resume");
        } else {
            panic!("Expected Resume");
        }
    }

    #[test]
    fn resume_with_append_sets_resume_message() {
        let mut job = test_job();
        job.vars.insert("prompt".into(), "Original".into());
        let agent = test_agent_def();
        let config = ActionConfig::with_append(AgentAction::Resume, "Try again.");
        let result = build_action_effects_for(&test_ctx(&agent, &config, "exit"), &job).unwrap();
        if let ActionEffects::Resume { input, resume, .. } = result {
            assert_eq!(input.get("resume_message"), Some(&"Try again.".to_string()));
            assert_eq!(input.get("prompt"), Some(&"Original".to_string()));
            assert!(!resume, "no prior session in test fixture");
        } else {
            panic!("Expected Resume");
        }
    }

    #[test]
    fn resume_without_message_uses_resume_session() {
        let mut job = test_job();
        job.step_history.push(oj_core::StepRecord {
            name: "execute".into(),
            started_at_ms: 0,
            finished_at_ms: None,
            outcome: oj_core::StepOutcome::Running,
            agent_id: Some("prev-session-uuid".into()),
            agent_name: Some("worker".into()),
        });
        let agent = test_agent_def();
        let config = ActionConfig::simple(AgentAction::Resume);
        let result = build_action_effects_for(&test_ctx(&agent, &config, "exit"), &job).unwrap();
        if let ActionEffects::Resume { resume, .. } = result {
            assert!(resume, "should resume when prior agent exists");
        } else {
            panic!("Expected Resume");
        }
    }

    #[test]
    fn resume_with_no_prior_session_falls_back() {
        let job = test_job();
        let agent = test_agent_def();
        let config = ActionConfig::simple(AgentAction::Resume);
        let result = build_action_effects_for(&test_ctx(&agent, &config, "exit"), &job).unwrap();
        if let ActionEffects::Resume { resume, .. } = result {
            assert!(!resume, "no prior session should not resume");
        } else {
            panic!("Expected Resume");
        }
    }

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

    #[yare::parameterized(
        missing_job  = { "nonexistent", "execute",    RUNBOOK_WITH_AGENT },
        missing_step = { "build",       "nonexistent", RUNBOOK_WITH_AGENT },
        non_agent    = { "build",       "execute",    RUNBOOK_WITHOUT_AGENT },
    )]
    fn get_agent_def_errors(kind: &str, step: &str, toml: &str) {
        let runbook = parse_runbook(toml).unwrap();
        let mut job = test_job();
        job.kind = kind.to_string();
        job.step = step.to_string();
        assert!(get_agent_def(&runbook, &job).is_err());
    }

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
}

// ── Property tests ────────────────────────────────────────────────────

mod properties {
    use crate::engine::lifecycle::{self, RunLifecycle};
    use crate::engine::monitor::*;
    use oj_core::{AgentError, AgentState, Crew, CrewStatus, Effect, Event, Job, StepStatus};
    use oj_runbook::{ActionConfig, AgentAction, AgentDef};
    use proptest::prelude::*;

    fn test_job() -> Job {
        let mut job = Job::builder().name("test").workspace_path("/tmp/test").build();
        // Add step_history so agent_id() resolves (Nudge/Ask need agent_id)
        job.step_history.push(oj_core::StepRecord {
            name: job.step.clone(),
            agent_id: Some("agent-1".to_string()),
            started_at_ms: 0,
            finished_at_ms: None,
            outcome: oj_core::StepOutcome::Running,
            agent_name: None,
        });
        job
    }
    fn test_ar() -> Crew {
        Crew::builder().build()
    }
    fn test_agent_def() -> AgentDef {
        AgentDef {
            name: "worker".into(),
            run: "claude".into(),
            prompt: Some("Do it.".into()),
            ..Default::default()
        }
    }
    fn ctx<'a>(def: &'a AgentDef, cfg: &'a ActionConfig, trigger: &'a str) -> ActionContext<'a> {
        ActionContext {
            agent_def: def,
            action_config: cfg,
            trigger,
            chain_pos: 0,
            questions: None,
            last_message: None,
        }
    }

    fn arb_action_no_gate() -> impl Strategy<Value = AgentAction> {
        prop_oneof![
            Just(AgentAction::Nudge),
            Just(AgentAction::Done),
            Just(AgentAction::Fail),
            Just(AgentAction::Resume),
            Just(AgentAction::Escalate),
            Just(AgentAction::Auto),
        ]
    }
    fn arb_trigger() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("idle".into()),
            Just("exit".into()),
            Just("error".into()),
            Just("on_idle".into()),
            Just("on_dead".into()),
            Just("prompt".into()),
            Just("prompt:question".into()),
            Just("idle:exhausted".into()),
            "[a-z_]{1,12}".prop_map(|s| s),
        ]
    }

    // ── Phase A: Action dispatch properties ──────────────────────────────────

    proptest! {
        #[test]
        fn action_maps_to_correct_variant(action in arb_action_no_gate(), trigger in arb_trigger()) {
            let def = test_agent_def();
            let cfg = ActionConfig::simple(action.clone());
            let check = |run: &dyn RunLifecycle| {
                let r = build_action_effects_for(&ctx(&def, &cfg, &trigger), run);
                let ok = match action {
                    AgentAction::Nudge    => matches!(r, Ok(ActionEffects::Nudge { .. })),
                    AgentAction::Done     => matches!(r, Ok(ActionEffects::Advance)),
                    AgentAction::Fail     => matches!(r, Ok(ActionEffects::Fail { .. })),
                    AgentAction::Resume   => matches!(r, Ok(ActionEffects::Resume { .. })),
                    AgentAction::Escalate => matches!(r, Ok(ActionEffects::Escalate { .. })),
                    AgentAction::Auto     => matches!(r, Ok(ActionEffects::Advance)),
                    AgentAction::Gate     => unreachable!(),
                };
                prop_assert!(ok, "wrong variant for {:?}", action);
                Ok(())
            };
            let job = test_job();
            check(&job)?;
            let run = test_ar();
            check(&run)?;
        }

        #[test]
        fn escalate_always_emits_decision_created(trigger in arb_trigger()) {
            let def = test_agent_def();
            let cfg = ActionConfig::simple(AgentAction::Escalate);
            let check = |run: &dyn RunLifecycle| -> Result<(), proptest::test_runner::TestCaseError> {
                let r = build_action_effects_for(&ctx(&def, &cfg, &trigger), run).unwrap();
                let ActionEffects::Escalate { effects } = r else {
                    prop_assert!(false, "expected Escalate");
                    return Ok(());
                };
                let has_dc = effects.iter().any(|e| matches!(e, Effect::Emit { event: Event::DecisionCreated { .. } }));
                prop_assert!(has_dc, "escalate must emit DecisionCreated");
                let has_n = effects.iter().any(|e| matches!(e, Effect::Notify { .. }));
                prop_assert!(has_n, "escalate must emit Notify");
                Ok(())
            };
            let job = test_job();
            check(&job)?;
            let run = test_ar();
            check(&run)?;
        }

        #[test]
        fn fail_uses_trigger_as_error(trigger in arb_trigger()) {
            let def = test_agent_def();
            let cfg = ActionConfig::simple(AgentAction::Fail);
            let check = |run: &dyn RunLifecycle| {
                if let Ok(ActionEffects::Fail { error }) = build_action_effects_for(&ctx(&def, &cfg, &trigger), run) {
                    prop_assert_eq!(error, trigger.clone());
                }
                Ok(())
            };
            let job = test_job();
            check(&job)?;
            let run = test_ar();
            check(&run)?;
        }

        #[test]
        fn gate_with_run_produces_gate(command in "[a-z]{1,15}") {
            let def = test_agent_def();
            let cfg = ActionConfig::WithOptions {
                action: AgentAction::Gate, message: None, append: false,
                run: Some(command.clone()), attempts: oj_runbook::Attempts::default(), cooldown: None,
            };
            let check = |run: &dyn RunLifecycle| {
                if let Ok(ActionEffects::Gate { command: cmd }) = build_action_effects_for(&ctx(&def, &cfg, "exit"), run) {
                    prop_assert_eq!(cmd, command.clone());
                } else {
                    prop_assert!(false, "expected Gate");
                }
                Ok(())
            };
            let job = test_job();
            check(&job)?;
            let run = test_ar();
            check(&run)?;
        }

        #[test]
        fn gate_without_run_always_errors(trigger in arb_trigger()) {
            let def = test_agent_def();
            let cfg = ActionConfig::simple(AgentAction::Gate);
            let job = test_job();
            prop_assert!(build_action_effects_for(&ctx(&def, &cfg, &trigger), &job).is_err());
            let run = test_ar();
            prop_assert!(build_action_effects_for(&ctx(&def, &cfg, &trigger), &run).is_err());
        }

        #[test]
        fn escalation_trigger_maps_consistently(trigger in arb_trigger()) {
            let def = test_agent_def();
            let cfg = ActionConfig::simple(AgentAction::Escalate);
            let job = test_job();
            let r = build_action_effects_for(&ctx(&def, &cfg, &trigger), &job).unwrap();
            if let ActionEffects::Escalate { effects } = r {
                let source = effects.iter().find_map(|e| match e {
                    Effect::Emit { event: Event::DecisionCreated { source, .. } } => Some(source.clone()),
                    _ => None,
                });
                prop_assert!(source.is_some(), "must have a source");
            }
        }
    }

    // ── Phase B: Guard invariant properties ──────────────────────────────────

    proptest! {
        #[test]
        fn escalation_always_cancels_exit_deferred(trigger in arb_trigger()) {
            let job = test_job();
            let effects = job.escalation_status_effects(&trigger, Some("d"));
            let has = effects.iter().any(|e| matches!(e, Effect::CancelTimer { .. }));
            prop_assert!(has, "job escalation must cancel timer");
            let run = test_ar();
            let effects = run.escalation_status_effects(&trigger, None);
            let has = effects.iter().any(|e| matches!(e, Effect::CancelTimer { .. }));
            prop_assert!(has, "run escalation must cancel timer");
        }

        #[test]
        fn job_escalation_emits_step_waiting(trigger in arb_trigger()) {
            let job = test_job();
            let effects = job.escalation_status_effects(&trigger, Some("d"));
            let has = effects.iter().any(|e| matches!(e, Effect::Emit { event: Event::StepWaiting { .. } }));
            prop_assert!(has, "job escalation must emit StepWaiting");
        }

        #[test]
        fn ar_escalation_emits_status_change(trigger in arb_trigger()) {
            let run = test_ar();
            let effects = run.escalation_status_effects(&trigger, None);
            let has = effects.iter().any(|e| matches!(e, Effect::Emit {
                event: Event::CrewUpdated { status: CrewStatus::Escalated, .. }
            }));
            prop_assert!(has, "run escalation must emit Escalated status");
        }

        #[test]
        fn monitor_state_preserves_exit_code(exit_code in any::<Option<i32>>()) {
            let state = MonitorState::from_agent_state(&AgentState::Exited { exit_code });
            match state {
                MonitorState::Exited { exit_code: actual } => prop_assert_eq!(actual, exit_code),
                _ => prop_assert!(false, "expected Exited"),
            }
        }

        #[test]
        fn notify_on_done_produces_effect(template in "[a-zA-Z ${}.]{1,30}") {
            let job = test_job();
            let mut def = test_agent_def();
            def.notify.on_done = Some(template.clone());
            let effect = lifecycle::notify_on_done(&job, &def);
            prop_assert!(effect.is_some());
            if let Some(Effect::Notify { title, .. }) = effect {
                prop_assert_eq!(title, "worker");
            }
        }
    }

    #[test]
    fn nudge_requires_agent() {
        let def = test_agent_def();
        let cfg = ActionConfig::simple(AgentAction::Nudge);
        let mut job = test_job();
        job.step_history.clear();
        assert!(build_action_effects_for(&ctx(&def, &cfg, "idle"), &job).is_err());
        let mut run = test_ar();
        run.agent_id = None;
        assert!(build_action_effects_for(&ctx(&def, &cfg, "idle"), &run).is_err());
    }

    #[yare::parameterized(
        completed = { StepStatus::Completed },
        failed    = { StepStatus::Failed },
        pending   = { StepStatus::Pending },
        running   = { StepStatus::Running },
    )]
    fn terminal_job_not_waiting(status: StepStatus) {
        let mut job = test_job();
        job.step_status = status;
        assert!(!job.is_waiting());
    }

    #[yare::parameterized(
        none    = { None },
        some_id = { Some("dec-123".to_string()) },
    )]
    fn waiting_job_is_waiting(decision_id: Option<String>) {
        let mut job = test_job();
        job.step_status = StepStatus::Waiting(decision_id);
        assert!(job.is_waiting());
    }

    #[yare::parameterized(
        starting  = { CrewStatus::Starting },
        running   = { CrewStatus::Running },
        completed = { CrewStatus::Completed },
        failed    = { CrewStatus::Failed },
    )]
    fn non_waiting_ar_not_waiting(status: CrewStatus) {
        let mut run = test_ar();
        run.status = status;
        assert!(!run.is_waiting());
    }

    #[yare::parameterized(
        escalated = { CrewStatus::Escalated },
        waiting   = { CrewStatus::Waiting },
    )]
    fn waiting_ar_is_waiting(status: CrewStatus) {
        let mut run = test_ar();
        run.status = status;
        assert!(run.is_waiting());
    }

    #[test]
    fn notify_without_template_is_none() {
        let job = test_job();
        let def = test_agent_def();
        assert!(lifecycle::notify_on_done(&job, &def).is_none());
        let run = test_ar();
        assert!(lifecycle::notify_on_done(&run, &def).is_none());
    }

    #[yare::parameterized(
        working   = { AgentState::Working,                             "Working" },
        idle      = { AgentState::WaitingForInput,                     "WaitingForInput" },
        exited_0  = { AgentState::Exited { exit_code: Some(0) },      "Exited" },
        exited_n  = { AgentState::Exited { exit_code: None },          "Exited" },
        gone      = { AgentState::SessionGone,                         "Gone" },
    )]
    fn monitor_state_from_agent_state(input: AgentState, expected_tag: &str) {
        let ms = MonitorState::from_agent_state(&input);
        let tag = match ms {
            MonitorState::Working => "Working",
            MonitorState::WaitingForInput => "WaitingForInput",
            MonitorState::Failed { .. } => "Failed",
            MonitorState::Exited { .. } => "Exited",
            MonitorState::Gone => "Gone",
            MonitorState::Prompting { .. } => "Prompting",
        };
        assert_eq!(tag, expected_tag);
    }

    #[yare::parameterized(
        unauthorized   = { AgentError::Unauthorized,                  Some(oj_runbook::ErrorType::Unauthorized) },
        out_of_credits = { AgentError::OutOfCredits,                  Some(oj_runbook::ErrorType::OutOfCredits) },
        no_internet    = { AgentError::NoInternet,                    Some(oj_runbook::ErrorType::NoInternet) },
        rate_limited   = { AgentError::RateLimited,                   Some(oj_runbook::ErrorType::RateLimited) },
        other          = { AgentError::Other("x".into()),             None },
    )]
    fn failure_maps_to_error_type(error: AgentError, expected: Option<oj_runbook::ErrorType>) {
        let state = MonitorState::from_agent_state(&AgentState::Failed(error));
        if let MonitorState::Failed { error_type, .. } = state {
            assert_eq!(error_type, expected);
        } else {
            panic!("expected Failed");
        }
    }

    #[yare::parameterized(
        with_zero     = { Some(0),   "agent exited (exit code: 0)" },
        with_one      = { Some(1),   "agent exited (exit code: 1)" },
        with_signal   = { Some(137), "agent exited (exit code: 137)" },
        with_none     = { None,      "agent exited" },
    )]
    fn exit_message_format(exit_code: Option<i32>, expected: &str) {
        assert_eq!(format_exit_message(exit_code), expected);
    }
}
