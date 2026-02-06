// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Epic runbook parsing tests.

use oj_runbook::{AgentAction, Attempts, PrimeDef, WorkspaceConfig, WorkspaceType};

fn parse_epic() -> oj_runbook::Runbook {
    super::parse_hcl(include_str!("../fixtures/epic.hcl"))
}

#[test]
fn command() {
    let cmd = &parse_epic().commands["epic"];
    assert!(cmd.run.is_job());
    assert_eq!(cmd.run.job_name(), Some("epic"));
    assert_eq!(cmd.args.positional.len(), 2);
    assert_eq!(cmd.args.positional[0].name, "name");
    assert_eq!(cmd.args.positional[1].name, "instructions");
    assert_eq!(cmd.args.options[0].name, "blocked-by");
    assert_eq!(cmd.defaults.get("blocked-by"), Some(&String::new()));
}

#[test]
fn job() {
    let job = &parse_epic().jobs["epic"];
    assert_eq!(job.name.as_deref(), Some("${var.name}"));
    assert_eq!(job.vars, vec!["name", "instructions", "blocked-by"]);
    assert_eq!(
        job.workspace,
        Some(WorkspaceConfig::Simple(WorkspaceType::Folder))
    );
    assert_eq!(job.steps.len(), 5);

    let steps: Vec<_> = job.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(steps, ["init", "decompose", "build", "submit", "cleanup"]);

    assert!(job.steps[0].run.is_shell());
    assert_eq!(
        job.steps[0].on_done.as_ref().map(|t| t.step_name()),
        Some("decompose")
    );
    assert!(job.steps[1].run.is_agent());
    assert_eq!(job.steps[1].agent_name(), Some("decompose"));
    assert!(job.steps[2].run.is_agent());
    assert_eq!(job.steps[2].agent_name(), Some("epic-builder"));
    assert!(job.steps[3].run.is_shell());
    assert!(job.steps[4].on_done.is_none()); // terminal step

    assert!(job.locals.contains_key("repo"));
    assert!(job.locals.contains_key("branch"));
    assert!(job.locals.contains_key("title"));
    assert!(job.notify.on_start.is_some());
    assert!(job.notify.on_done.is_some());
    assert!(job.notify.on_fail.is_some());
}

#[test]
fn decompose_agent() {
    let agent = parse_epic().get_agent("decompose").unwrap().clone();
    assert!(agent.run.contains("claude"));
    assert!(agent.run.contains("--disallowed-tools"));
    assert!(agent.prompt.is_some());
    assert_eq!(agent.on_idle.action(), &AgentAction::Gate);
    assert_eq!(agent.on_idle.run(), Some("test -s .epic-root-id"));
    assert_eq!(agent.on_dead.action(), &AgentAction::Fail);
    match &agent.prime {
        Some(PrimeDef::Commands(cmds)) => assert_eq!(cmds.len(), 5),
        other => panic!("expected PrimeDef::Commands, got {:?}", other),
    }
}

#[test]
fn builder_agent() {
    let agent = parse_epic().get_agent("epic-builder").unwrap().clone();
    assert!(agent.run.contains("claude"));
    assert!(agent.prompt.is_some());
    assert_eq!(agent.on_idle.action(), &AgentAction::Gate);
    assert_eq!(agent.on_idle.attempts(), Attempts::Forever);
    assert_eq!(agent.on_dead.action(), &AgentAction::Resume);
    assert!(agent.on_dead.append());
    assert!(agent.on_dead.message().is_some());
    match &agent.prime {
        Some(PrimeDef::Commands(cmds)) => assert_eq!(cmds.len(), 5),
        other => panic!("expected PrimeDef::Commands, got {:?}", other),
    }
}
