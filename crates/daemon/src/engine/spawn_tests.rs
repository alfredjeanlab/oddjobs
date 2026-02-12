// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for agent spawning

use super::*;
use crate::engine::test_helpers::spawn_effects;
use oj_core::{AgentId, JobId, OwnerId};
use oj_runbook::PrimeDef;
use std::collections::HashMap;
use tempfile::TempDir;

/// Extracts SpawnAgent fields from the first effect, panicking if not SpawnAgent.
struct SpawnAgent<'a> {
    agent_id: &'a AgentId,
    agent_name: &'a str,
    owner: &'a OwnerId,
    command: &'a str,
    cwd: &'a Option<PathBuf>,
    env: &'a [(String, String)],
    input: &'a HashMap<String, String>,
}

fn unwrap_spawn_agent(effects: &[Effect]) -> SpawnAgent<'_> {
    match &effects[0] {
        Effect::SpawnAgent { agent_id, agent_name, owner, command, cwd, env, input, .. } => {
            SpawnAgent { agent_id, agent_name, owner, command, cwd, env, input }
        }
        other => panic!("Expected SpawnAgent effect, got: {}", other.name()),
    }
}

fn test_job() -> Job {
    Job::builder()
        .id("job-1")
        .name("test-feature")
        .cwd("/tmp/workspace")
        .workspace_path("/tmp/workspace")
        .build()
}

fn test_agent_def() -> AgentDef {
    AgentDef {
        name: "worker".to_string(),
        run: "claude --print \"${prompt}\"".to_string(),
        prompt: Some("Do the task: ${name}".to_string()),
        ..Default::default()
    }
}

#[test]
fn build_spawn_effects_creates_agent_and_timer() {
    let workspace = TempDir::new().unwrap();
    let agent = test_agent_def();
    let job = test_job();
    let input: HashMap<String, String> =
        [("prompt".to_string(), "Build feature".to_string())].into_iter().collect();

    let pid = JobId::new("job-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "worker",
        &input,
        workspace.path(),
        workspace.path(),
        false,
    )
    .unwrap();

    // Should produce 1 effect: SpawnAgent (liveness timer is set by handle_session_created)
    assert_eq!(effects.len(), 1);

    // Only effect is SpawnAgent
    assert!(matches!(&effects[0], Effect::SpawnAgent { .. }));
}

#[yare::parameterized(
    interpolates_variables = {
        "Do the task: ${name}",
        &[("prompt", "Build feature")],
        "Do the task: test-feature",
        &[],
        &[]
    },
    namespaces_job_inputs = {
        "Task: ${var.prompt}",
        &[("prompt", "Add authentication")],
        "Task: Add authentication",
        &[("var.prompt", "Add authentication"), ("prompt", "Task: Add authentication")],
        &[]
    },
    nested_namespace = {
        "Fix: ${var.bug.title} (id: ${var.bug.id})",
        &[("bug.title", "Button color wrong"), ("bug.id", "proj-abc1")],
        "Fix: Button color wrong (id: proj-abc1)",
        &[("var.bug.title", "Button color wrong"), ("var.bug.id", "proj-abc1")],
        &["bug.title"]
    },
    locals_in_prompt = {
        "Branch: ${local.branch}, Title: ${local.title}",
        &[("local.branch", "fix/bug-123"), ("local.title", "fix: button color"), ("name", "my-fix")],
        "Branch: fix/bug-123, Title: fix: button color",
        &[],
        &[]
    },
)]
fn prompt_interpolation(
    prompt_template: &str,
    input_pairs: &[(&str, &str)],
    expected_command_fragment: &str,
    expected_inputs: &[(&str, &str)],
    absent_keys: &[&str],
) {
    let workspace = TempDir::new().unwrap();
    let agent = AgentDef {
        name: "worker".to_string(),
        run: "claude --print \"${prompt}\"".to_string(),
        prompt: Some(prompt_template.to_string()),
        ..Default::default()
    };
    let job = test_job();
    let input: HashMap<String, String> =
        input_pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();

    let pid = JobId::new("job-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "worker",
        &input,
        workspace.path(),
        workspace.path(),
        false,
    )
    .unwrap();

    let sa = unwrap_spawn_agent(&effects);
    assert!(
        sa.command.contains(expected_command_fragment),
        "Expected command to contain '{}', got: {}",
        expected_command_fragment,
        sa.command
    );
    for (key, val) in expected_inputs {
        assert_eq!(sa.input.get(*key), Some(&val.to_string()), "expected input '{}'", key);
    }
    for key in absent_keys {
        assert!(sa.input.get(*key).is_none(), "key '{}' should be absent", key);
    }
}

#[yare::parameterized(
    absolute = { "/absolute/path", false },
    relative = { "subdir",         true },
)]
fn build_spawn_effects_cwd_handling(cwd_input: &str, is_relative: bool) {
    let workspace = TempDir::new().unwrap();
    let mut agent = test_agent_def();
    agent.cwd = Some(cwd_input.to_string());
    let job = test_job();

    let pid = JobId::new("job-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    let sa = unwrap_spawn_agent(&effects);
    let expected =
        if is_relative { workspace.path().join(cwd_input) } else { PathBuf::from(cwd_input) };
    assert_eq!(sa.cwd.as_ref().unwrap(), &expected);
}

#[test]
fn build_spawn_effects_fails_on_missing_prompt_file() {
    let workspace = TempDir::new().unwrap();
    let mut agent = test_agent_def();
    agent.prompt = None;
    agent.prompt_file = Some(PathBuf::from("/nonexistent/prompt.txt"));
    let job = test_job();

    let pid = JobId::new("job-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let result = spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path());

    // Should fail due to missing prompt file
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("prompt"));
}

#[test]
fn build_spawn_effects_carries_full_config() {
    let workspace = TempDir::new().unwrap();
    let agent = test_agent_def();
    let job = test_job();
    let input: HashMap<String, String> =
        [("prompt".to_string(), "Build feature".to_string())].into_iter().collect();

    let pid = JobId::new("job-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "worker",
        &input,
        workspace.path(),
        workspace.path(),
        false,
    )
    .unwrap();

    // SpawnAgent should carry command, env, and cwd
    let sa = unwrap_spawn_agent(&effects);
    // agent_id is now a UUID
    assert!(
        uuid::Uuid::parse_str(sa.agent_id.as_str()).is_ok(),
        "agent_id should be a valid UUID: {}",
        sa.agent_id
    );
    assert_eq!(sa.agent_name, "worker");
    assert_eq!(sa.owner, &OwnerId::Job(JobId::new("job-1")));
    assert!(!sa.command.is_empty());
    assert!(sa.cwd.is_some());
    // System vars are not namespaced
    assert!(sa.input.contains_key("job_id"));
    assert!(sa.input.contains_key("name"));
    assert!(sa.input.contains_key("workspace"));
    // Job vars are namespaced under "var."
    assert!(sa.input.contains_key("var.prompt"));
    // Rendered prompt is added as "prompt"
    assert!(sa.input.contains_key("prompt"));
}

#[test]
fn build_spawn_effects_returns_only_spawn_agent() {
    let workspace = TempDir::new().unwrap();
    let agent = test_agent_def();
    let job = test_job();

    let pid = JobId::new("job-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    // Liveness timer is now set by handle_session_created, not build_spawn_effects
    assert_eq!(effects.len(), 1);
    assert!(matches!(&effects[0], Effect::SpawnAgent { .. }));
}

#[test]
fn build_spawn_effects_escapes_backticks_in_prompt() {
    let workspace = TempDir::new().unwrap();
    // Agent prompt contains backticks (like markdown code references)
    let agent = AgentDef {
        name: "worker".to_string(),
        run: "claude \"${prompt}\"".to_string(),
        prompt: Some("Write to `plans/${name}.md`".to_string()),
        ..Default::default()
    };
    let job = test_job();

    let pid = JobId::new("job-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    let sa = unwrap_spawn_agent(&effects);
    // Backticks should be escaped to prevent shell command substitution
    assert!(
        sa.command.contains("\\`plans/test-feature.md\\`"),
        "Expected escaped backticks, got: {}",
        sa.command
    );
}

#[yare::parameterized(
    commands = { PrimeDef::Commands(vec!["echo hello".into(), "git status".into()]) },
    script   = { PrimeDef::Script("echo ${name} ${workspace}".into()) },
)]
fn build_spawn_effects_with_prime_succeeds(prime: PrimeDef) {
    let workspace = TempDir::new().unwrap();
    let mut agent = test_agent_def();
    agent.prime = Some(prime);
    let job = test_job();

    let pid = JobId::new("job-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    assert_eq!(effects.len(), 1);
    assert!(matches!(&effects[0], Effect::SpawnAgent { .. }));
}

#[test]
fn build_spawn_effects_standalone_agent_carries_crew_id() {
    let workspace = TempDir::new().unwrap();
    let agent = AgentDef {
        name: "fixer".to_string(),
        run: "claude --print \"${prompt}\"".to_string(),
        prompt: Some("Fix: ${var.description}".to_string()),
        ..Default::default()
    };
    let input: HashMap<String, String> =
        [("description".to_string(), "broken button".to_string())].into_iter().collect();

    let crew_id = oj_core::CrewId::new("run-test-1");
    let ctx = SpawnCtx::from_crew(&crew_id, "fixer", "");
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "fixer",
        &input,
        workspace.path(),
        workspace.path(),
        false,
    )
    .unwrap();

    // SpawnAgent should carry the crew_id as owner
    let sa = unwrap_spawn_agent(&effects);
    assert_eq!(sa.owner, &OwnerId::Crew(oj_core::CrewId::new("run-test-1")));
    // Command args should be accessible via var. project
    assert_eq!(sa.input.get("var.description"), Some(&"broken button".to_string()));
    // Prompt should be interpolated with the var
    assert!(
        sa.command.contains("Fix: broken button"),
        "Expected interpolated prompt, got: {}",
        sa.command
    );
}

#[test]
fn build_spawn_effects_always_passes_oj_state_dir() {
    let workspace = TempDir::new().unwrap();
    let state_dir = TempDir::new().unwrap();
    let agent = test_agent_def();
    let job = test_job();

    // Ensure OJ_STATE_DIR is NOT set in the current environment
    // (simulates daemon that resolved state_dir via XDG_STATE_HOME or $HOME fallback)
    std::env::remove_var("OJ_STATE_DIR");

    let pid = JobId::new("job-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), state_dir.path()).unwrap();

    let sa = unwrap_spawn_agent(&effects);
    let oj_state = sa.env.iter().find(|(k, _)| k == "OJ_STATE_DIR").map(|(_, v)| v.as_str());
    assert_eq!(
        oj_state,
        Some(state_dir.path().to_str().unwrap()),
        "OJ_STATE_DIR must always be passed from state_dir parameter, \
         not conditionally from env var"
    );
}

#[test]
fn build_spawn_effects_trims_trailing_newlines_from_command() {
    let workspace = TempDir::new().unwrap();
    // Simulate a heredoc-style run command with trailing newline (as from HCL <<-CMD)
    // The bug: if trailing newline isn't trimmed, appended args become a separate command
    let agent = AgentDef {
        name: "worker".to_string(),
        // Trailing newline from heredoc - if not trimmed, appended prompt would be on new line
        run: "claude --model opus\n".to_string(),
        prompt: Some("Do the task".to_string()),
        ..Default::default()
    };
    let job = test_job();

    let pid = JobId::new("job-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    let sa = unwrap_spawn_agent(&effects);
    // Prompt should be on the same line as the base command (no bare newline between)
    // A well-formed command: "claude --model opus \"Do the task: test-feature\""
    // A broken command: newline before the prompt making it a separate command
    assert!(
        !sa.command.contains("\n\""),
        "trailing newline should be trimmed so appended args don't become separate command: {}",
        sa.command
    );
    // Verify the command is properly formed
    assert!(
        sa.command.starts_with("claude --model opus \""),
        "command should have no embedded newlines before appended args: {}",
        sa.command
    );
}
