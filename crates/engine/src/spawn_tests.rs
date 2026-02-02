// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for agent spawning

use super::*;
use oj_core::{PipelineId, StepStatus};
use oj_runbook::PrimeDef;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use tempfile::TempDir;

fn test_pipeline() -> Pipeline {
    Pipeline {
        id: "pipe-1".to_string(),
        name: "test-feature".to_string(),
        kind: "build".to_string(),
        step: "execute".to_string(),
        step_status: StepStatus::Running,
        runbook_hash: "testhash".to_string(),
        cwd: PathBuf::from("/tmp/workspace"),
        session_id: None,
        workspace_id: None,
        workspace_path: Some(PathBuf::from("/tmp/workspace")),
        vars: HashMap::new(),
        created_at: Instant::now(),
        step_started_at: Instant::now(),
        error: None,
        step_history: Vec::new(),
        action_attempts: HashMap::new(),
        agent_signal: None,
        namespace: String::new(),
    }
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
    let pipeline = test_pipeline();
    let input: HashMap<String, String> = [("prompt".to_string(), "Build feature".to_string())]
        .into_iter()
        .collect();

    let pid = PipelineId::new("pipe-1");
    let effects =
        build_spawn_effects(&agent, &pipeline, &pid, "worker", &input, workspace.path()).unwrap();

    // Should produce 2 effects: SpawnAgent, SetTimer
    assert_eq!(effects.len(), 2);

    // First effect is SpawnAgent
    assert!(matches!(&effects[0], Effect::SpawnAgent { .. }));

    // Second effect is SetTimer for liveness monitoring
    assert!(matches!(&effects[1], Effect::SetTimer { id, .. } if id.is_liveness()));
}

#[test]
fn build_spawn_effects_interpolates_variables() {
    let workspace = TempDir::new().unwrap();
    let agent = test_agent_def();
    let pipeline = test_pipeline();
    let input: HashMap<String, String> = [("prompt".to_string(), "Build feature".to_string())]
        .into_iter()
        .collect();

    let pid = PipelineId::new("pipe-1");
    let effects =
        build_spawn_effects(&agent, &pipeline, &pid, "worker", &input, workspace.path()).unwrap();

    // Check the SpawnAgent effect has interpolated command
    // The command uses ${prompt} which now gets the rendered agent prompt
    // Agent prompt is "Do the task: ${name}" where ${name} is pipeline.name ("test-feature")
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
    let pipeline = test_pipeline();

    let pid = PipelineId::new("pipe-1");
    let effects = build_spawn_effects(
        &agent,
        &pipeline,
        &pid,
        "worker",
        &HashMap::new(),
        workspace.path(),
    )
    .unwrap();

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
    let pipeline = test_pipeline();

    let pid = PipelineId::new("pipe-1");
    let effects = build_spawn_effects(
        &agent,
        &pipeline,
        &pid,
        "worker",
        &HashMap::new(),
        workspace.path(),
    )
    .unwrap();

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
    let pipeline = test_pipeline();

    let pid = PipelineId::new("pipe-1");
    build_spawn_effects(
        &agent,
        &pipeline,
        &pid,
        "worker",
        &HashMap::new(),
        workspace.path(),
    )
    .unwrap();

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
    let pipeline = test_pipeline();

    let pid = PipelineId::new("pipe-1");
    let result = build_spawn_effects(
        &agent,
        &pipeline,
        &pid,
        "worker",
        &HashMap::new(),
        workspace.path(),
    );

    // Should fail due to missing prompt file
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("prompt"));
}

#[test]
fn build_spawn_effects_carries_full_config() {
    let workspace = TempDir::new().unwrap();
    let agent = test_agent_def();
    let pipeline = test_pipeline();
    let input: HashMap<String, String> = [("prompt".to_string(), "Build feature".to_string())]
        .into_iter()
        .collect();

    let pid = PipelineId::new("pipe-1");
    let effects =
        build_spawn_effects(&agent, &pipeline, &pid, "worker", &input, workspace.path()).unwrap();

    // SpawnAgent should carry command, env, and cwd
    if let Effect::SpawnAgent {
        agent_id,
        agent_name,
        pipeline_id,
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
        assert_eq!(pipeline_id, "pipe-1");
        assert!(!command.is_empty());
        assert!(cwd.is_some());
        // System vars are not namespaced
        assert!(effect_inputs.contains_key("pipeline_id"));
        assert!(effect_inputs.contains_key("name"));
        assert!(effect_inputs.contains_key("workspace"));
        // Pipeline vars are namespaced under "var."
        assert!(effect_inputs.contains_key("var.prompt"));
        // Rendered prompt is added as "prompt"
        assert!(effect_inputs.contains_key("prompt"));
    } else {
        panic!("Expected SpawnAgent effect");
    }
}

#[test]
fn build_spawn_effects_timer_uses_liveness_interval() {
    let workspace = TempDir::new().unwrap();
    let agent = test_agent_def();
    let pipeline = test_pipeline();

    let pid = PipelineId::new("pipe-1");
    let effects = build_spawn_effects(
        &agent,
        &pipeline,
        &pid,
        "worker",
        &HashMap::new(),
        workspace.path(),
    )
    .unwrap();

    if let Effect::SetTimer { id, duration } = &effects[1] {
        assert_eq!(id, "liveness:pipe-1");
        assert_eq!(*duration, LIVENESS_INTERVAL);
    } else {
        panic!("Expected SetTimer effect");
    }
}

#[test]
fn build_spawn_effects_namespaces_pipeline_inputs() {
    let workspace = TempDir::new().unwrap();
    // Agent uses ${var.prompt} to access pipeline vars
    let agent = AgentDef {
        name: "worker".to_string(),
        run: "claude --print \"${prompt}\"".to_string(),
        prompt: Some("Task: ${var.prompt}".to_string()),
        ..Default::default()
    };
    let pipeline = test_pipeline();
    let input: HashMap<String, String> = [("prompt".to_string(), "Add authentication".to_string())]
        .into_iter()
        .collect();

    let pid = PipelineId::new("pipe-1");
    let effects =
        build_spawn_effects(&agent, &pipeline, &pid, "worker", &input, workspace.path()).unwrap();

    if let Effect::SpawnAgent {
        command,
        input: effect_inputs,
        ..
    } = &effects[0]
    {
        // Pipeline vars are namespaced under "var."
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
    let pipeline = test_pipeline();
    let input: HashMap<String, String> = [
        ("bug.title".to_string(), "Button color wrong".to_string()),
        ("bug.id".to_string(), "proj-abc1".to_string()),
    ]
    .into_iter()
    .collect();

    let pid = PipelineId::new("pipe-1");
    let effects =
        build_spawn_effects(&agent, &pipeline, &pid, "worker", &input, workspace.path()).unwrap();

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
fn escape_for_shell_double_quotes_escapes_backticks() {
    let result = escape_for_shell_double_quotes("Write to `file.txt`");
    assert_eq!(result, "Write to \\`file.txt\\`");
}

#[test]
fn escape_for_shell_double_quotes_escapes_all_special_chars() {
    let result = escape_for_shell_double_quotes(r#"$VAR `cmd` "quote" \slash"#);
    assert_eq!(result, r#"\$VAR \`cmd\` \"quote\" \\slash"#);
}

#[test]
fn escape_for_shell_double_quotes_preserves_normal_text() {
    let result = escape_for_shell_double_quotes("Normal text with newlines\nand tabs\t");
    assert_eq!(result, "Normal text with newlines\nand tabs\t");
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
    let pipeline = test_pipeline();

    let pid = PipelineId::new("pipe-1");
    let effects = build_spawn_effects(
        &agent,
        &pipeline,
        &pid,
        "worker",
        &HashMap::new(),
        workspace.path(),
    )
    .unwrap();

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
    let pipeline = test_pipeline();

    let pid = PipelineId::new("pipe-1");
    let effects = build_spawn_effects(
        &agent,
        &pipeline,
        &pid,
        "worker",
        &HashMap::new(),
        workspace.path(),
    )
    .unwrap();

    // Should still produce 2 effects: SpawnAgent, SetTimer
    assert_eq!(effects.len(), 2);
    assert!(matches!(&effects[0], Effect::SpawnAgent { .. }));
}

#[test]
fn build_spawn_effects_with_prime_script_succeeds() {
    let workspace = TempDir::new().unwrap();
    let mut agent = test_agent_def();
    agent.prime = Some(PrimeDef::Script("echo ${name} ${workspace}".to_string()));
    let pipeline = test_pipeline();

    let pid = PipelineId::new("pipe-prime-1");
    let effects = build_spawn_effects(
        &agent,
        &pipeline,
        &pid,
        "worker",
        &HashMap::new(),
        workspace.path(),
    )
    .unwrap();

    // Should produce standard effects
    assert_eq!(effects.len(), 2);
    assert!(matches!(&effects[0], Effect::SpawnAgent { .. }));
    assert!(matches!(&effects[1], Effect::SetTimer { .. }));
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
    let pipeline = test_pipeline();
    let input: HashMap<String, String> = [
        ("local.branch".to_string(), "fix/bug-123".to_string()),
        ("local.title".to_string(), "fix: button color".to_string()),
        ("name".to_string(), "my-fix".to_string()),
    ]
    .into_iter()
    .collect();

    let pid = PipelineId::new("pipe-1");
    let effects =
        build_spawn_effects(&agent, &pipeline, &pid, "worker", &input, workspace.path()).unwrap();

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
