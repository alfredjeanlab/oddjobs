// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::{parse_runbook, parse_runbook_with_format, Format, ParseError, QueueType};

mod action_trigger;
mod prime;

// New format - uses RunDirective tables and args string syntax
const SAMPLE_RUNBOOK_NEW: &str = r#"
[command.build]
args = "<name> <prompt>"
run = { pipeline = "build" }
[command.build.defaults]
branch = "main"

[pipeline.build]
vars  = ["name", "prompt"]

[[pipeline.build.step]]
name = "init"
run = "git worktree add worktrees/${name} -b feature/${name}"

[[pipeline.build.step]]
name = "plan"
run = { agent = "planner" }

[[pipeline.build.step]]
name = "execute"
run = { agent = "executor" }
on_done = "done"
on_fail = "failed"

[[pipeline.build.step]]
name = "done"
run = "echo done"

[[pipeline.build.step]]
name = "failed"
run = "echo failed"

[agent.planner]
run = "claude -p"
prompt = "Plan: ${var.prompt}"
[agent.planner.env]
OJ_STEP = "plan"

[agent.executor]
run = "claude \"${prompt}\""
cwd = "worktrees/${name}"
"#;

#[test]
fn parse_new_format_runbook() {
    let runbook = parse_runbook(SAMPLE_RUNBOOK_NEW).unwrap();

    // Commands
    assert!(runbook.commands.contains_key("build"));
    let cmd = &runbook.commands["build"];
    assert!(cmd.run.is_pipeline());
    assert_eq!(cmd.run.pipeline_name(), Some("build"));
    assert_eq!(cmd.args.positional.len(), 2);
    assert_eq!(cmd.args.positional[0].name, "name");
    assert_eq!(cmd.args.positional[1].name, "prompt");
    assert_eq!(cmd.defaults.get("branch"), Some(&"main".to_string()));

    // Pipelines
    assert!(runbook.pipelines.contains_key("build"));
    let pipeline = &runbook.pipelines["build"];
    assert_eq!(pipeline.vars, vec!["name", "prompt"]);
    assert_eq!(pipeline.steps.len(), 5);

    // Step checks
    assert_eq!(pipeline.steps[0].name, "init");
    assert!(pipeline.steps[0].run.is_shell());

    assert_eq!(pipeline.steps[1].name, "plan");
    assert!(pipeline.steps[1].run.is_agent());
    assert_eq!(pipeline.steps[1].agent_name(), Some("planner"));

    // Agents
    assert!(runbook.agents.contains_key("planner"));
    let agent = &runbook.agents["planner"];
    assert!(agent.run.contains("claude"));
    assert!(agent.env.contains_key("OJ_STEP"));
}

#[test]
fn parse_empty_runbook() {
    let runbook = parse_runbook("").unwrap();
    assert!(runbook.commands.is_empty());
    assert!(runbook.pipelines.is_empty());
}

#[test]
fn parse_command_with_args_string() {
    let toml = r#"
[command.deploy]
args = "<env> [-t/--tag <version>] [-f/--force] [targets...]"
run = "deploy.sh"
[command.deploy.defaults]
tag = "latest"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let cmd = &runbook.commands["deploy"];

    assert_eq!(cmd.args.positional.len(), 1);
    assert_eq!(cmd.args.positional[0].name, "env");
    assert_eq!(cmd.args.options.len(), 1);
    assert_eq!(cmd.args.options[0].name, "tag");
    assert_eq!(cmd.args.flags.len(), 1);
    assert_eq!(cmd.args.flags[0].name, "force");
    assert!(cmd.args.variadic.is_some());
    assert_eq!(cmd.args.variadic.as_ref().unwrap().name, "targets");

    assert!(cmd.run.is_shell());
    assert_eq!(cmd.run.shell_command(), Some("deploy.sh"));
}

// ============================================================================
// Error Tests: Missing Required Fields
// ============================================================================

#[test]
fn error_missing_command_run() {
    let toml = r#"
[command.build]
args = "<name>"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::Toml(_)));
    let msg = err.to_string();
    assert!(
        msg.contains("run"),
        "error should mention 'run' field: {}",
        msg
    );
}

#[test]
fn error_missing_step_name() {
    let toml = r#"
[pipeline.test]
[[pipeline.test.step]]
run = "echo test"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(msg.contains("name"), "error should mention 'name': {}", msg);
}

#[test]
fn error_missing_step_run() {
    let toml = r#"
[pipeline.test]
[[pipeline.test.step]]
name = "build"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::Toml(_)));
    let msg = err.to_string();
    assert!(
        msg.contains("run"),
        "error should mention 'run' field: {}",
        msg
    );
}

// ============================================================================
// Error Tests: Invalid Shell Commands
// ============================================================================

#[test]
fn error_unterminated_quote_in_command_run() {
    let toml = r#"
[command.test]
run = "echo 'unterminated"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("command.test.run"),
        "error should mention location: {}",
        msg
    );
}

#[test]
fn error_unterminated_subshell_in_step() {
    let toml = r#"
[pipeline.test]
[[pipeline.test.step]]
name = "broken"
run = "echo $(incomplete"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("pipeline.test.step[0](broken).run"),
        "error should mention location: {}",
        msg
    );
}

#[test]
fn error_unterminated_quote_in_agent_run() {
    let toml = r#"
[agent.test]
run = "claude \"unterminated"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("agent.test.run"),
        "error should mention location: {}",
        msg
    );
}

// ============================================================================
// Error Tests: Invalid TOML Structure
// ============================================================================

#[test]
fn error_command_not_table() {
    let toml = r#"
[command]
build = "not a table"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::Toml(_)));
}

#[test]
fn error_invalid_run_directive() {
    let toml = r#"
[command.test]
run = { invalid = "key" }
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::Toml(_)));
}

#[test]
fn error_pipeline_not_table() {
    let toml = r#"
[pipeline]
build = "not a table"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::Toml(_)));
}

// ============================================================================
// Error Tests: Invalid Argument Specs
// ============================================================================

#[test]
fn error_duplicate_arg_name() {
    let toml = r#"
[command.test]
args = "<name> <name>"
run = "test.sh"
"#;
    let err = parse_runbook(toml).unwrap_err();
    // ArgSpec errors come through serde's custom deserializer
    assert!(matches!(err, ParseError::Toml(_)));
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate") || msg.contains("name"),
        "error should mention duplicate: {}",
        msg
    );
}

#[test]
fn error_variadic_not_last() {
    let toml = r#"
[command.test]
args = "<files...> <extra>"
run = "test.sh"
"#;
    let err = parse_runbook(toml).unwrap_err();
    // ArgSpec errors come through serde's custom deserializer
    assert!(matches!(err, ParseError::Toml(_)));
    let msg = err.to_string();
    assert!(
        msg.contains("variadic"),
        "error should mention variadic: {}",
        msg
    );
}

#[test]
fn error_optional_before_required() {
    let toml = r#"
[command.test]
args = "[opt] <req>"
run = "test.sh"
"#;
    let err = parse_runbook(toml).unwrap_err();
    // ArgSpec errors come through serde's custom deserializer
    assert!(matches!(err, ParseError::Toml(_)));
}

// ============================================================================
// Error Tests: Agent-Specific Errors
// ============================================================================

#[test]
fn error_agent_missing_run() {
    let toml = r#"
[agent.test]
prompt = "Do something"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::Toml(_)));
    let msg = err.to_string();
    assert!(
        msg.contains("run"),
        "error should mention 'run' field: {}",
        msg
    );
}

#[test]
fn error_unrecognized_agent_command() {
    let toml = r#"
[agent.test]
run = "unknown-tool -p 'do something'"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("unrecognized"),
        "error should mention 'unrecognized': {}",
        msg
    );
    assert!(
        msg.contains("unknown-tool"),
        "error should mention the command name: {}",
        msg
    );
}

#[test]
fn parse_agent_with_claude_command() {
    let toml = r#"
[agent.planner]
run = "claude --print 'Plan something'"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.agents.contains_key("planner"));
}

#[test]
fn parse_agent_with_claudeless_command() {
    let toml = r#"
[agent.runner]
run = "claudeless --scenario 'Run tests'"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.agents.contains_key("runner"));
}

#[test]
fn parse_agent_with_absolute_path() {
    let toml = r#"
[agent.planner]
run = "/usr/local/bin/claude --print 'Plan something'"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.agents.contains_key("planner"));
}

#[test]
fn error_unrecognized_absolute_path() {
    let toml = r#"
[agent.test]
run = "/usr/bin/codex --help"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("codex"),
        "error should mention the command basename: {}",
        msg
    );
}

// ============================================================================
// Prompt Configuration Tests
// ============================================================================

#[test]
fn parse_agent_prompt_field_no_inline() {
    // prompt field configured, no ${prompt} in run - valid (system appends prompt)
    let toml = r#"
[agent.plan]
run = "claude --dangerously-skip-permissions"
prompt = "Plan the feature"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.agents.contains_key("plan"));
}

#[test]
fn parse_agent_prompt_file_no_inline() {
    // prompt_file configured, no ${prompt} in run - valid (system appends prompt)
    let toml = r#"
[agent.plan]
run = "claude --dangerously-skip-permissions"
prompt_file = "prompts/plan.md"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.agents.contains_key("plan"));
}

#[test]
fn error_agent_prompt_field_with_positional() {
    // prompt field configured AND positional arg in run - error (conflict)
    let toml = r#"
[agent.plan]
run = "claude --print \"${prompt}\""
prompt = "Plan the feature"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("positional"),
        "error should mention positional arguments: {}",
        msg
    );
}

#[test]
fn parse_agent_no_prompt_no_reference() {
    let toml = r#"
[agent.plan]
run = "claude --dangerously-skip-permissions"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.agents.contains_key("plan"));
}

#[test]
fn parse_agent_prompt_reference_without_field() {
    // ${prompt} in run without a prompt field is valid — the value comes from pipeline input
    let toml = r#"
[agent.plan]
run = "claude -p \"${prompt}\""
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.agents.contains_key("plan"));
}

#[test]
fn error_agent_session_id_rejected() {
    // --session-id is rejected (system adds it automatically)
    let toml = r#"
[agent.plan]
run = "claude --session-id abc123"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("session-id"),
        "error should mention session-id: {}",
        msg
    );
    assert!(
        msg.contains("automatically"),
        "error should mention automatic addition: {}",
        msg
    );
}

#[test]
fn error_agent_session_id_equals_rejected() {
    // --session-id=value form is also rejected
    let toml = r#"
[agent.plan]
run = "claude --session-id=abc123"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("session-id"),
        "error should mention session-id: {}",
        msg
    );
}

// ============================================================================
// JSON Format Tests
// ============================================================================

const SAMPLE_JSON_RUNBOOK: &str = r#"
{
  "command": {
    "build": {
      "args": "<name> <prompt>",
      "run": { "pipeline": "build" },
      "defaults": {
        "branch": "main"
      }
    }
  },
  "pipeline": {
    "build": {
      "input": ["name", "prompt"],
      "step": [
        {
          "name": "init",
          "run": "git worktree add worktrees/${name} -b feature/${name}"
        },
        {
          "name": "plan",
          "run": { "agent": "planner" }
        },
        {
          "name": "execute",
          "run": { "agent": "executor" },
          "on_done": "done",
          "on_fail": "failed"
        },
        {
          "name": "done",
          "run": "echo done"
        },
        {
          "name": "failed",
          "run": "echo failed"
        }
      ]
    }
  },
  "agent": {
    "planner": {
      "run": "claude -p \"Plan: ${prompt}\"",
      "env": {
        "OJ_STEP": "plan"
      }
    },
    "executor": {
      "run": "claude \"${prompt}\"",
      "cwd": "worktrees/${name}"
    }
  }
}
"#;

#[test]
fn parse_json_runbook() {
    let runbook = parse_runbook_with_format(SAMPLE_JSON_RUNBOOK, Format::Json).unwrap();

    // Commands
    assert!(runbook.commands.contains_key("build"));
    let cmd = &runbook.commands["build"];
    assert!(cmd.run.is_pipeline());
    assert_eq!(cmd.run.pipeline_name(), Some("build"));
    assert_eq!(cmd.args.positional.len(), 2);
    assert_eq!(cmd.args.positional[0].name, "name");
    assert_eq!(cmd.args.positional[1].name, "prompt");
    assert_eq!(cmd.defaults.get("branch"), Some(&"main".to_string()));

    // Pipelines
    assert!(runbook.pipelines.contains_key("build"));
    let pipeline = &runbook.pipelines["build"];
    assert_eq!(pipeline.vars, vec!["name", "prompt"]);
    assert_eq!(pipeline.steps.len(), 5);

    assert_eq!(pipeline.steps[0].name, "init");
    assert!(pipeline.steps[0].run.is_shell());

    assert_eq!(pipeline.steps[1].name, "plan");
    assert!(pipeline.steps[1].run.is_agent());
    assert_eq!(pipeline.steps[1].agent_name(), Some("planner"));

    assert_eq!(pipeline.steps[2].name, "execute");
    assert_eq!(
        pipeline.steps[2].on_done.as_ref().map(|t| t.step_name()),
        Some("done")
    );
    assert_eq!(
        pipeline.steps[2].on_fail.as_ref().map(|t| t.step_name()),
        Some("failed")
    );

    assert_eq!(pipeline.steps[3].name, "done");
    assert_eq!(pipeline.steps[4].name, "failed");

    // Agents
    assert!(runbook.agents.contains_key("planner"));
    let agent = &runbook.agents["planner"];
    assert!(agent.run.contains("claude"));
    assert!(agent.env.contains_key("OJ_STEP"));
}

#[test]
fn parse_json_empty_runbook() {
    let runbook = parse_runbook_with_format("{}", Format::Json).unwrap();
    assert!(runbook.commands.is_empty());
    assert!(runbook.pipelines.is_empty());
}

// ============================================================================
// HCL Format Tests
// ============================================================================

const SAMPLE_HCL_RUNBOOK: &str = r#"
command "build" {
  args = "<name> <prompt>"
  run  = { pipeline = "build" }

  defaults = {
    branch = "main"
  }
}

pipeline "build" {
  vars  = ["name", "prompt"]

  step "init" {
    run = "git worktree add worktrees/${name} -b feature/${name}"
  }

  step "plan" {
    run = { agent = "planner" }
  }

  step "execute" {
    run     = { agent = "executor" }
    on_done = "done"
    on_fail = "failed"
  }

  step "done" {
    run = "echo done"
  }

  step "failed" {
    run = "echo failed"
  }
}

agent "planner" {
  run = "claude -p \"Plan: ${prompt}\""

  env = {
    OJ_STEP = "plan"
  }
}

agent "executor" {
  run = "claude \"${prompt}\""
  cwd = "worktrees/${name}"
}
"#;

#[test]
fn parse_hcl_runbook() {
    let runbook = parse_runbook_with_format(SAMPLE_HCL_RUNBOOK, Format::Hcl).unwrap();

    // Commands
    assert!(runbook.commands.contains_key("build"));
    let cmd = &runbook.commands["build"];
    assert!(cmd.run.is_pipeline());
    assert_eq!(cmd.run.pipeline_name(), Some("build"));
    assert_eq!(cmd.args.positional.len(), 2);
    assert_eq!(cmd.args.positional[0].name, "name");
    assert_eq!(cmd.args.positional[1].name, "prompt");
    assert_eq!(cmd.defaults.get("branch"), Some(&"main".to_string()));

    // Pipelines
    assert!(runbook.pipelines.contains_key("build"));
    let pipeline = &runbook.pipelines["build"];
    assert_eq!(pipeline.vars, vec!["name", "prompt"]);
    assert_eq!(pipeline.steps.len(), 5);

    // Steps get name from block label
    assert_eq!(pipeline.steps[0].name, "init");
    assert!(pipeline.steps[0].run.is_shell());

    assert_eq!(pipeline.steps[1].name, "plan");
    assert!(pipeline.steps[1].run.is_agent());
    assert_eq!(pipeline.steps[1].agent_name(), Some("planner"));

    assert_eq!(pipeline.steps[2].name, "execute");
    assert_eq!(
        pipeline.steps[2].on_done.as_ref().map(|t| t.step_name()),
        Some("done")
    );
    assert_eq!(
        pipeline.steps[2].on_fail.as_ref().map(|t| t.step_name()),
        Some("failed")
    );

    assert_eq!(pipeline.steps[3].name, "done");
    assert_eq!(pipeline.steps[4].name, "failed");

    // Agents
    assert!(runbook.agents.contains_key("planner"));
    let agent = &runbook.agents["planner"];
    assert!(agent.run.contains("claude"));
    assert!(agent.env.contains_key("OJ_STEP"));
}

#[test]
fn parse_hcl_empty_runbook() {
    let runbook = parse_runbook_with_format("", Format::Hcl).unwrap();
    assert!(runbook.commands.is_empty());
    assert!(runbook.pipelines.is_empty());
}

#[test]
fn parse_hcl_step_names_from_block_labels() {
    let hcl = r#"
pipeline "deploy" {
  vars  = ["env"]

  step "build" {
    run = "make build"
  }

  step "test" {
    run = "make test"
    on_done = "deploy"
  }

  step "deploy" {
    run = "make deploy"
  }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let pipeline = &runbook.pipelines["deploy"];
    assert_eq!(pipeline.steps.len(), 3);
    assert_eq!(pipeline.steps[0].name, "build");
    assert_eq!(pipeline.steps[1].name, "test");
    assert_eq!(
        pipeline.steps[1].on_done.as_ref().map(|t| t.step_name()),
        Some("deploy")
    );
    assert_eq!(pipeline.steps[2].name, "deploy");
}

#[test]
fn parse_hcl_agent_validation() {
    let hcl = r#"
agent "planner" {
  run = "claude --print 'Plan something'"
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    assert!(runbook.agents.contains_key("planner"));
}

#[test]
fn error_hcl_unrecognized_agent_command() {
    let hcl = r#"
agent "test" {
  run = "unknown-tool -p 'do something'"
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("unrecognized"),
        "error should mention 'unrecognized': {}",
        msg
    );
}

// ============================================================================
// Queue Type Tests
// ============================================================================

#[test]
fn parse_external_queue_with_explicit_type() {
    let hcl = r#"
queue "bugs" {
  type = "external"
  list = "wok list -t bug -o json"
  take = "wok start ${item.id}"
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let queue = &runbook.queues["bugs"];
    assert_eq!(queue.queue_type, QueueType::External);
    assert_eq!(queue.list.as_deref(), Some("wok list -t bug -o json"));
    assert_eq!(queue.take.as_deref(), Some("wok start ${item.id}"));
}

#[test]
fn parse_persisted_queue() {
    let hcl = r#"
queue "merges" {
  type     = "persisted"
  vars     = ["branch", "title", "base"]
  defaults = { base = "main" }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let queue = &runbook.queues["merges"];
    assert_eq!(queue.queue_type, QueueType::Persisted);
    assert_eq!(queue.vars, vec!["branch", "title", "base"]);
    assert_eq!(queue.defaults.get("base"), Some(&"main".to_string()));
    assert!(queue.list.is_none());
    assert!(queue.take.is_none());
}

#[test]
fn parse_queue_defaults_to_external() {
    let hcl = r#"
queue "items" {
  list = "echo '[]'"
  take = "echo ok"
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let queue = &runbook.queues["items"];
    assert_eq!(queue.queue_type, QueueType::External);
}

#[test]
fn error_external_queue_missing_list() {
    let hcl = r#"
queue "items" {
  type = "external"
  take = "echo ok"
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("external queue requires 'list' field"),
        "error should mention missing list: {}",
        msg
    );
}

#[test]
fn error_external_queue_missing_take() {
    let hcl = r#"
queue "items" {
  type = "external"
  list = "echo '[]'"
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("external queue requires 'take' field"),
        "error should mention missing take: {}",
        msg
    );
}

#[test]
fn error_persisted_queue_missing_vars() {
    let hcl = r#"
queue "items" {
  type = "persisted"
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("persisted queue requires 'vars' field"),
        "error should mention missing vars: {}",
        msg
    );
}

#[test]
fn error_persisted_queue_with_list() {
    let hcl = r#"
queue "items" {
  type = "persisted"
  vars = ["branch"]
  list = "echo '[]'"
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("persisted queue must not have 'list' field"),
        "error should mention forbidden list: {}",
        msg
    );
}

#[test]
fn error_persisted_queue_with_take() {
    let hcl = r#"
queue "items" {
  type = "persisted"
  vars = ["branch"]
  take = "echo ok"
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("persisted queue must not have 'take' field"),
        "error should mention forbidden take: {}",
        msg
    );
}
