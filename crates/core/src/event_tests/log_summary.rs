// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for `Event::log_summary()` — agent, command, job, runbook, session,
//! shell, step, and system events.

use super::*;

#[test]
fn log_summary_agent_state_events() {
    let cases = vec![
        (
            Event::AgentWorking {
                agent_id: AgentId::new("a1"),
                owner: OwnerId::Job(JobId::default()),
            },
            "agent:working agent=a1",
        ),
        (
            Event::AgentWaiting {
                agent_id: AgentId::new("a2"),
                owner: OwnerId::Job(JobId::default()),
            },
            "agent:waiting agent=a2",
        ),
        (
            Event::AgentFailed {
                agent_id: AgentId::new("a3"),
                error: AgentError::RateLimited,
                owner: OwnerId::Job(JobId::default()),
            },
            "agent:failed agent=a3",
        ),
        (
            Event::AgentExited {
                agent_id: AgentId::new("a4"),
                exit_code: Some(0),
                owner: OwnerId::Job(JobId::default()),
            },
            "agent:exited agent=a4",
        ),
        (
            Event::AgentGone {
                agent_id: AgentId::new("a5"),
                owner: OwnerId::Job(JobId::default()),
            },
            "agent:gone agent=a5",
        ),
    ];
    for (event, expected) in cases {
        assert_eq!(event.log_summary(), expected, "failed for {:?}", event);
    }
}

#[test]
fn log_summary_agent_input() {
    let event = Event::AgentInput {
        agent_id: AgentId::new("a1"),
        input: "hello".to_string(),
    };
    assert_eq!(event.log_summary(), "agent:input agent=a1");
}

#[test]
fn log_summary_agent_signal() {
    let event = Event::AgentSignal {
        agent_id: AgentId::new("a1"),
        kind: AgentSignalKind::Complete,
        message: Some("done".to_string()),
    };
    assert_eq!(event.log_summary(), "agent:signal id=a1 kind=Complete");

    let event = Event::AgentSignal {
        agent_id: AgentId::new("a2"),
        kind: AgentSignalKind::Escalate,
        message: None,
    };
    assert_eq!(event.log_summary(), "agent:signal id=a2 kind=Escalate");
}

#[test]
fn log_summary_agent_idle() {
    let event = Event::AgentIdle {
        agent_id: AgentId::new("a1"),
    };
    assert_eq!(event.log_summary(), "agent:idle agent=a1");
}

#[test]
fn log_summary_agent_stop() {
    let event = Event::AgentStop {
        agent_id: AgentId::new("a1"),
    };
    assert_eq!(event.log_summary(), "agent:stop agent=a1");
}

#[test]
fn log_summary_agent_prompt() {
    let event = Event::AgentPrompt {
        agent_id: AgentId::new("a1"),
        prompt_type: PromptType::Permission,
        question_data: None,
    };
    assert_eq!(
        event.log_summary(),
        "agent:prompt agent=a1 prompt_type=Permission"
    );
}

#[test]
fn log_summary_command_run_no_namespace() {
    let event = Event::CommandRun {
        job_id: JobId::new("j1"),
        job_name: "build".to_string(),
        project_root: PathBuf::from("/proj"),
        invoke_dir: PathBuf::from("/proj"),
        command: "build".to_string(),
        args: HashMap::new(),
        namespace: String::new(),
    };
    assert_eq!(event.log_summary(), "command:run id=j1 cmd=build");
}

#[test]
fn log_summary_command_run_with_namespace() {
    let event = Event::CommandRun {
        job_id: JobId::new("j1"),
        job_name: "build".to_string(),
        project_root: PathBuf::from("/proj"),
        invoke_dir: PathBuf::from("/proj"),
        command: "deploy".to_string(),
        args: HashMap::new(),
        namespace: "myns".to_string(),
    };
    assert_eq!(event.log_summary(), "command:run id=j1 ns=myns cmd=deploy");
}

#[test]
fn log_summary_job_created_no_namespace() {
    let event = Event::JobCreated {
        id: JobId::new("j1"),
        kind: "build".to_string(),
        name: "test".to_string(),
        runbook_hash: "abc".to_string(),
        cwd: PathBuf::from("/"),
        vars: HashMap::new(),
        initial_step: "init".to_string(),
        created_at_epoch_ms: 0,
        namespace: String::new(),
        cron_name: None,
    };
    assert_eq!(
        event.log_summary(),
        "job:created id=j1 kind=build name=test"
    );
}

#[test]
fn log_summary_job_created_with_namespace() {
    let event = Event::JobCreated {
        id: JobId::new("j1"),
        kind: "build".to_string(),
        name: "test".to_string(),
        runbook_hash: "abc".to_string(),
        cwd: PathBuf::from("/"),
        vars: HashMap::new(),
        initial_step: "init".to_string(),
        created_at_epoch_ms: 0,
        namespace: "prod".to_string(),
        cron_name: None,
    };
    assert_eq!(
        event.log_summary(),
        "job:created id=j1 ns=prod kind=build name=test"
    );
}

#[test]
fn log_summary_job_advanced() {
    let event = Event::JobAdvanced {
        id: JobId::new("j1"),
        step: "deploy".to_string(),
    };
    assert_eq!(event.log_summary(), "job:advanced id=j1 step=deploy");
}

#[test]
fn log_summary_job_updated() {
    let event = Event::JobUpdated {
        id: JobId::new("j1"),
        vars: HashMap::new(),
    };
    assert_eq!(event.log_summary(), "job:updated id=j1");
}

#[test]
fn log_summary_job_resume() {
    let event = Event::JobResume {
        id: JobId::new("j1"),
        message: None,
        vars: HashMap::new(),
        kill: false,
    };
    assert_eq!(event.log_summary(), "job:resume id=j1");
}

#[test]
fn log_summary_job_cancelling_cancel_deleted() {
    assert_eq!(
        Event::JobCancelling {
            id: JobId::new("j1")
        }
        .log_summary(),
        "job:cancelling id=j1"
    );
    assert_eq!(
        Event::JobCancel {
            id: JobId::new("j2")
        }
        .log_summary(),
        "job:cancel id=j2"
    );
    assert_eq!(
        Event::JobDeleted {
            id: JobId::new("j3")
        }
        .log_summary(),
        "job:deleted id=j3"
    );
}

#[test]
fn log_summary_runbook_loaded() {
    let runbook = serde_json::json!({
        "agents": {"builder": {}, "tester": {}},
        "jobs": {"ci": {}}
    });
    let event = Event::RunbookLoaded {
        hash: "abcdef1234567890".to_string(),
        version: 3,
        runbook,
    };
    assert_eq!(
        event.log_summary(),
        "runbook:loaded hash=abcdef123456 v=3 agents=2 jobs=1"
    );
}

#[test]
fn log_summary_runbook_loaded_empty() {
    let runbook = serde_json::json!({});
    let event = Event::RunbookLoaded {
        hash: "short".to_string(),
        version: 1,
        runbook,
    };
    assert_eq!(
        event.log_summary(),
        "runbook:loaded hash=short v=1 agents=0 jobs=0"
    );
}

#[test]
fn log_summary_session_created_job_owner() {
    use crate::owner::OwnerId;
    let event = Event::SessionCreated {
        id: SessionId::new("s1"),
        owner: OwnerId::Job(JobId::new("j1")),
    };
    assert_eq!(event.log_summary(), "session:created id=s1 job=j1");
}

#[test]
fn log_summary_session_created_agent_run_owner() {
    use crate::agent_run::AgentRunId;
    use crate::owner::OwnerId;
    let event = Event::SessionCreated {
        id: SessionId::new("s1"),
        owner: OwnerId::AgentRun(AgentRunId::new("ar1")),
    };
    assert_eq!(event.log_summary(), "session:created id=s1 agent_run=ar1");
}

#[test]
fn log_summary_session_input_deleted() {
    assert_eq!(
        Event::SessionInput {
            id: SessionId::new("s1"),
            input: "text".to_string(),
        }
        .log_summary(),
        "session:input id=s1"
    );
    assert_eq!(
        Event::SessionDeleted {
            id: SessionId::new("s2"),
        }
        .log_summary(),
        "session:deleted id=s2"
    );
}

#[test]
fn log_summary_shell_exited() {
    let event = Event::ShellExited {
        job_id: JobId::new("j1"),
        step: "init".to_string(),
        exit_code: 42,
        stdout: None,
        stderr: None,
    };
    assert_eq!(event.log_summary(), "shell:exited job=j1 step=init exit=42");
}

#[test]
fn log_summary_step_events() {
    assert_eq!(
        Event::StepStarted {
            job_id: JobId::new("j1"),
            step: "build".to_string(),
            agent_id: None,
            agent_name: None,
        }
        .log_summary(),
        "step:started job=j1 step=build"
    );
    assert_eq!(
        Event::StepWaiting {
            job_id: JobId::new("j1"),
            step: "review".to_string(),
            reason: Some("gate failed".to_string()),
            decision_id: None,
        }
        .log_summary(),
        "step:waiting job=j1 step=review"
    );
    assert_eq!(
        Event::StepCompleted {
            job_id: JobId::new("j1"),
            step: "deploy".to_string(),
        }
        .log_summary(),
        "step:completed job=j1 step=deploy"
    );
    assert_eq!(
        Event::StepFailed {
            job_id: JobId::new("j1"),
            step: "test".to_string(),
            error: "oops".to_string(),
        }
        .log_summary(),
        "step:failed job=j1 step=test"
    );
}

#[test]
fn log_summary_shutdown_and_custom() {
    assert_eq!(Event::Shutdown.log_summary(), "system:shutdown");
    assert_eq!(Event::Custom.log_summary(), "custom");
}
