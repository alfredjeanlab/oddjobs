// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for agent spawning

use super::*;
use crate::test_helpers::spawn_effects;
use oj_core::{JobId, OwnerId};
use oj_runbook::PrimeDef;
use std::collections::HashMap;
use tempfile::TempDir;

fn test_job() -> Job {
    Job::builder()
        .id("pipe-1")
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
    let input: HashMap<String, String> = [("prompt".to_string(), "Build feature".to_string())]
        .into_iter()
        .collect();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "worker",
        &input,
        workspace.path(),
        workspace.path(),
        None,
    )
    .unwrap();

    // Should produce 1 effect: SpawnAgent (liveness timer is set by handle_session_created)
    assert_eq!(effects.len(), 1);

    // Only effect is SpawnAgent
    assert!(matches!(&effects[0], Effect::SpawnAgent { .. }));
}

#[test]
fn build_spawn_effects_interpolates_variables() {
    let workspace = TempDir::new().unwrap();
    let agent = test_agent_def();
    let job = test_job();
    let input: HashMap<String, String> = [("prompt".to_string(), "Build feature".to_string())]
        .into_iter()
        .collect();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "worker",
        &input,
        workspace.path(),
        workspace.path(),
        None,
    )
    .unwrap();

    // Check the SpawnAgent effect has interpolated command
    // The command uses ${prompt} which now gets the rendered agent prompt
    // Agent prompt is "Do the task: ${name}" where ${name} is job.name ("test-feature")
    if let Effect::SpawnAgent { command, .. } = &effects[0] {
        // Command should have the rendered prompt interpolated
        assert!(command.contains("Do the task: test-feature"));
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_uses_absolute_cwd() {
    let workspace = TempDir::new().unwrap();
    let mut agent = test_agent_def();
    agent.cwd = Some("/absolute/path".to_string());
    let job = test_job();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    if let Effect::SpawnAgent { cwd, .. } = &effects[0] {
        assert_eq!(cwd.as_ref().unwrap(), &PathBuf::from("/absolute/path"));
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_uses_relative_cwd() {
    let workspace = TempDir::new().unwrap();
    let mut agent = test_agent_def();
    agent.cwd = Some("subdir".to_string());
    let job = test_job();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    if let Effect::SpawnAgent { cwd, .. } = &effects[0] {
        assert_eq!(cwd.as_ref().unwrap(), &workspace.path().join("subdir"));
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_prepares_workspace() {
    let workspace = TempDir::new().unwrap();
    let agent = test_agent_def();
    let job = test_job();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    // Workspace should not have CLAUDE.md (that comes from the worktree, not workspace prep)
    let claude_md = workspace.path().join("CLAUDE.md");
    assert!(
        !claude_md.exists(),
        "Should not overwrite project CLAUDE.md"
    );
}

#[test]
fn build_spawn_effects_fails_on_missing_prompt_file() {
    let workspace = TempDir::new().unwrap();
    let mut agent = test_agent_def();
    agent.prompt = None;
    agent.prompt_file = Some(PathBuf::from("/nonexistent/prompt.txt"));
    let job = test_job();

    let pid = JobId::new("pipe-1");
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
    let input: HashMap<String, String> = [("prompt".to_string(), "Build feature".to_string())]
        .into_iter()
        .collect();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "worker",
        &input,
        workspace.path(),
        workspace.path(),
        None,
    )
    .unwrap();

    // SpawnAgent should carry command, env, and cwd
    if let Effect::SpawnAgent {
        agent_id,
        agent_name,
        owner,
        command,
        cwd,
        input: effect_inputs,
        ..
    } = &effects[0]
    {
        // agent_id is now a UUID
        assert!(
            uuid::Uuid::parse_str(agent_id.as_str()).is_ok(),
            "agent_id should be a valid UUID: {}",
            agent_id
        );
        assert_eq!(agent_name, "worker");
        assert_eq!(owner, &OwnerId::Job(JobId::new("pipe-1")));
        assert!(!command.is_empty());
        assert!(cwd.is_some());
        // System vars are not namespaced
        assert!(effect_inputs.contains_key("job_id"));
        assert!(effect_inputs.contains_key("name"));
        assert!(effect_inputs.contains_key("workspace"));
        // Job vars are namespaced under "var."
        assert!(effect_inputs.contains_key("var.prompt"));
        // Rendered prompt is added as "prompt"
        assert!(effect_inputs.contains_key("prompt"));
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_returns_only_spawn_agent() {
    let workspace = TempDir::new().unwrap();
    let agent = test_agent_def();
    let job = test_job();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    // Liveness timer is now set by handle_session_created, not build_spawn_effects
    assert_eq!(effects.len(), 1);
    assert!(matches!(&effects[0], Effect::SpawnAgent { .. }));
}

#[test]
fn build_spawn_effects_namespaces_job_inputs() {
    let workspace = TempDir::new().unwrap();
    // Agent uses ${var.prompt} to access job vars
    let agent = AgentDef {
        name: "worker".to_string(),
        run: "claude --print \"${prompt}\"".to_string(),
        prompt: Some("Task: ${var.prompt}".to_string()),
        ..Default::default()
    };
    let job = test_job();
    let input: HashMap<String, String> = [("prompt".to_string(), "Add authentication".to_string())]
        .into_iter()
        .collect();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "worker",
        &input,
        workspace.path(),
        workspace.path(),
        None,
    )
    .unwrap();

    if let Effect::SpawnAgent {
        command,
        input: effect_inputs,
        ..
    } = &effects[0]
    {
        // Job vars are namespaced under "var."
        assert_eq!(
            effect_inputs.get("var.prompt"),
            Some(&"Add authentication".to_string())
        );
        // Rendered prompt is added as "prompt" (shell-escaped)
        assert_eq!(
            effect_inputs.get("prompt"),
            Some(&"Task: Add authentication".to_string())
        );
        // Command gets the rendered prompt
        assert!(command.contains("Task: Add authentication"));
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_inputs_namespace_in_prompt() {
    let workspace = TempDir::new().unwrap();
    // Agent prompt uses ${var.bug.title} namespace
    let agent = AgentDef {
        name: "worker".to_string(),
        run: "claude --print \"${prompt}\"".to_string(),
        prompt: Some("Fix: ${var.bug.title} (id: ${var.bug.id})".to_string()),
        ..Default::default()
    };
    let job = test_job();
    let input: HashMap<String, String> = [
        ("bug.title".to_string(), "Button color wrong".to_string()),
        ("bug.id".to_string(), "proj-abc1".to_string()),
    ]
    .into_iter()
    .collect();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "worker",
        &input,
        workspace.path(),
        workspace.path(),
        None,
    )
    .unwrap();

    if let Effect::SpawnAgent {
        command,
        input: effect_inputs,
        ..
    } = &effects[0]
    {
        // Only namespaced keys should be available
        assert_eq!(
            effect_inputs.get("var.bug.title"),
            Some(&"Button color wrong".to_string())
        );
        assert_eq!(
            effect_inputs.get("var.bug.id"),
            Some(&"proj-abc1".to_string())
        );
        // Bare keys should NOT be available
        assert!(effect_inputs.get("bug.title").is_none());
        // Rendered prompt should use the namespaced keys
        assert!(
            command.contains("Fix: Button color wrong (id: proj-abc1)"),
            "Expected interpolated prompt, got: {}",
            command
        );
    } else {
        panic!("Expected SpawnAgent effect");
    }
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

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    if let Effect::SpawnAgent { command, .. } = &effects[0] {
        // Backticks should be escaped to prevent shell command substitution
        assert!(
            command.contains("\\`plans/test-feature.md\\`"),
            "Expected escaped backticks, got: {}",
            command
        );
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_with_prime_succeeds() {
    let workspace = TempDir::new().unwrap();
    let mut agent = test_agent_def();
    agent.prime = Some(PrimeDef::Commands(vec![
        "echo hello".to_string(),
        "git status".to_string(),
    ]));
    let job = test_job();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    // Should still produce 1 effect: SpawnAgent
    assert_eq!(effects.len(), 1);
    assert!(matches!(&effects[0], Effect::SpawnAgent { .. }));
}

#[test]
fn build_spawn_effects_with_prime_script_succeeds() {
    let workspace = TempDir::new().unwrap();
    let mut agent = test_agent_def();
    agent.prime = Some(PrimeDef::Script("echo ${name} ${workspace}".to_string()));
    let job = test_job();

    let pid = JobId::new("pipe-prime-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    // Should produce 1 effect: SpawnAgent
    assert_eq!(effects.len(), 1);
    assert!(matches!(&effects[0], Effect::SpawnAgent { .. }));
}

#[test]
fn build_spawn_effects_exposes_locals_in_prompt() {
    let workspace = TempDir::new().unwrap();
    let agent = AgentDef {
        name: "worker".to_string(),
        run: "claude --print \"${prompt}\"".to_string(),
        prompt: Some("Branch: ${local.branch}, Title: ${local.title}".to_string()),
        ..Default::default()
    };
    let job = test_job();
    let input: HashMap<String, String> = [
        ("local.branch".to_string(), "fix/bug-123".to_string()),
        ("local.title".to_string(), "fix: button color".to_string()),
        ("name".to_string(), "my-fix".to_string()),
    ]
    .into_iter()
    .collect();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "worker",
        &input,
        workspace.path(),
        workspace.path(),
        None,
    )
    .unwrap();

    if let Effect::SpawnAgent { command, .. } = &effects[0] {
        assert!(
            command.contains("Branch: fix/bug-123, Title: fix: button color"),
            "Expected locals in prompt, got: {}",
            command
        );
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_standalone_agent_carries_agent_run_id() {
    let workspace = TempDir::new().unwrap();
    let agent = AgentDef {
        name: "fixer".to_string(),
        run: "claude --print \"${prompt}\"".to_string(),
        prompt: Some("Fix: ${var.description}".to_string()),
        ..Default::default()
    };
    let input: HashMap<String, String> = [("description".to_string(), "broken button".to_string())]
        .into_iter()
        .collect();

    let agent_run_id = oj_core::AgentRunId::new("ar-test-1");
    let ctx = SpawnCtx::from_agent_run(&agent_run_id, "fixer", "");
    let effects = build_spawn_effects(
        &agent,
        &ctx,
        "fixer",
        &input,
        workspace.path(),
        workspace.path(),
        None,
    )
    .unwrap();

    // SpawnAgent should carry the agent_run_id as owner
    if let Effect::SpawnAgent {
        owner,
        command,
        input: effect_inputs,
        ..
    } = &effects[0]
    {
        assert_eq!(
            owner,
            &OwnerId::AgentRun(oj_core::AgentRunId::new("ar-test-1"))
        );
        // Command args should be accessible via var. namespace
        assert_eq!(
            effect_inputs.get("var.description"),
            Some(&"broken button".to_string())
        );
        // Prompt should be interpolated with the var
        assert!(
            command.contains("Fix: broken button"),
            "Expected interpolated prompt, got: {}",
            command
        );
    } else {
        panic!("Expected SpawnAgent effect");
    }
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

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), state_dir.path()).unwrap();

    if let Effect::SpawnAgent { env, .. } = &effects[0] {
        let oj_state = env
            .iter()
            .find(|(k, _)| k == "OJ_STATE_DIR")
            .map(|(_, v)| v.as_str());
        assert_eq!(
            oj_state,
            Some(state_dir.path().to_str().unwrap()),
            "OJ_STATE_DIR must always be passed from state_dir parameter, \
             not conditionally from env var"
        );
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

// =============================================================================
// User Env File Injection Tests
// =============================================================================

#[test]
fn build_spawn_effects_injects_user_env_vars() {
    let workspace = TempDir::new().unwrap();
    let state_dir = TempDir::new().unwrap();

    // Write a global env file
    let mut global = std::collections::BTreeMap::new();
    global.insert("MY_TOKEN".to_string(), "secret123".to_string());
    global.insert("MY_URL".to_string(), "https://example.com".to_string());
    crate::env::write_env_file(&crate::env::global_env_path(state_dir.path()), &global).unwrap();

    let agent = test_agent_def();
    let mut job = test_job();
    job.namespace = "testproject".to_string();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), state_dir.path()).unwrap();

    if let Effect::SpawnAgent { env, .. } = &effects[0] {
        let env_map: HashMap<&str, &str> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(env_map.get("MY_TOKEN"), Some(&"secret123"));
        assert_eq!(env_map.get("MY_URL"), Some(&"https://example.com"));
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_user_env_does_not_override_system_vars() {
    let workspace = TempDir::new().unwrap();
    let state_dir = TempDir::new().unwrap();

    // Write a global env file that tries to override OJ_NAMESPACE
    let mut global = std::collections::BTreeMap::new();
    global.insert("OJ_NAMESPACE".to_string(), "hacked".to_string());
    global.insert("MY_VAR".to_string(), "ok".to_string());
    crate::env::write_env_file(&crate::env::global_env_path(state_dir.path()), &global).unwrap();

    let agent = test_agent_def();
    let mut job = test_job();
    job.namespace = "real-ns".to_string();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), state_dir.path()).unwrap();

    if let Effect::SpawnAgent { env, .. } = &effects[0] {
        let env_map: HashMap<&str, &str> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        // System var should NOT be overridden by user env file
        assert_eq!(env_map.get("OJ_NAMESPACE"), Some(&"real-ns"));
        // Regular user var should be present
        assert_eq!(env_map.get("MY_VAR"), Some(&"ok"));
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_trims_trailing_newlines_from_command() {
    let workspace = TempDir::new().unwrap();
    // Simulate a heredoc-style run command with trailing newline (as from HCL <<-CMD)
    // The bug: if trailing newline isn't trimmed, appended args become a separate command
    let agent = AgentDef {
        name: "worker".to_string(),
        // Trailing newline from heredoc - if not trimmed, --session-id would be on new line
        run: "claude --model opus\n".to_string(),
        prompt: Some("Do the task".to_string()),
        ..Default::default()
    };
    let job = test_job();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), workspace.path()).unwrap();

    if let Effect::SpawnAgent { command, .. } = &effects[0] {
        // --session-id should be on the same line as the base command (no bare newline between)
        // A well-formed command would be: "claude --model opus --session-id xxx ..."
        // A broken command would have newline before --session-id making it a separate command
        assert!(
            !command.contains("\n--session-id") && !command.contains("\n --session-id"),
            "trailing newline should be trimmed so appended args don't become separate command: {}",
            command
        );
        // Verify the command is properly formed
        assert!(
            command.starts_with("claude --model opus --session-id"),
            "command should have no embedded newlines before appended args: {}",
            command
        );
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_project_env_overrides_global() {
    let workspace = TempDir::new().unwrap();
    let state_dir = TempDir::new().unwrap();

    // Global env
    let mut global = std::collections::BTreeMap::new();
    global.insert("TOKEN".to_string(), "global-val".to_string());
    global.insert("GLOBAL_ONLY".to_string(), "here".to_string());
    crate::env::write_env_file(&crate::env::global_env_path(state_dir.path()), &global).unwrap();

    // Project env overrides TOKEN
    let mut project = std::collections::BTreeMap::new();
    project.insert("TOKEN".to_string(), "project-val".to_string());
    crate::env::write_env_file(
        &crate::env::project_env_path(state_dir.path(), "myns"),
        &project,
    )
    .unwrap();

    let agent = test_agent_def();
    let mut job = test_job();
    job.namespace = "myns".to_string();

    let pid = JobId::new("pipe-1");
    let ctx = SpawnCtx::from_job(&job, &pid);
    let effects =
        spawn_effects(&agent, &ctx, "worker", workspace.path(), state_dir.path()).unwrap();

    if let Effect::SpawnAgent { env, .. } = &effects[0] {
        let env_map: HashMap<&str, &str> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(env_map.get("TOKEN"), Some(&"project-val"));
        assert_eq!(env_map.get("GLOBAL_ONLY"), Some(&"here"));
    } else {
        panic!("Expected SpawnAgent effect");
    }
}
