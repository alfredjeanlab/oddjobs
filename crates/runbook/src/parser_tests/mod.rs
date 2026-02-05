// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::{parse_runbook, parse_runbook_with_format, Format, ParseError, QueueType};

mod action_trigger;
mod cron;
mod prime;
mod queue_poll;
mod references;
mod template_refs;

// New format - uses RunDirective tables and args string syntax
const SAMPLE_RUNBOOK_NEW: &str = r#"
[command.build]
args = "<name> <prompt>"
run = { job = "build" }
[command.build.defaults]
branch = "main"

[job.build]
vars  = ["name", "prompt"]

[[job.build.step]]
name = "init"
run = "git worktree add worktrees/${name} -b feature/${name}"
on_done = "plan"

[[job.build.step]]
name = "plan"
run = { agent = "planner" }
on_done = "execute"

[[job.build.step]]
name = "execute"
run = { agent = "executor" }
on_done = "done"
on_fail = "failed"

[[job.build.step]]
name = "done"
run = "echo done"

[[job.build.step]]
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
    assert!(cmd.run.is_job());
    assert_eq!(cmd.run.job_name(), Some("build"));
    assert_eq!(cmd.args.positional.len(), 2);
    assert_eq!(cmd.args.positional[0].name, "name");
    assert_eq!(cmd.args.positional[1].name, "prompt");
    assert_eq!(cmd.defaults.get("branch"), Some(&"main".to_string()));

    // Jobs
    assert!(runbook.jobs.contains_key("build"));
    let job = &runbook.jobs["build"];
    assert_eq!(job.vars, vec!["name", "prompt"]);
    assert_eq!(job.steps.len(), 5);

    // Step checks
    assert_eq!(job.steps[0].name, "init");
    assert!(job.steps[0].run.is_shell());

    assert_eq!(job.steps[1].name, "plan");
    assert!(job.steps[1].run.is_agent());
    assert_eq!(job.steps[1].agent_name(), Some("planner"));

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
    assert!(runbook.jobs.is_empty());
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
[job.test]
[[job.test.step]]
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
[job.test]
[[job.test.step]]
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
[job.test]
[[job.test.step]]
name = "broken"
run = "echo $(incomplete"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("job.test.step[0](broken).run"),
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
fn error_job_not_table() {
    let toml = r#"
[job]
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
    // ${prompt} in run without a prompt field is valid — the value comes from job input
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
      "run": { "job": "build" },
      "defaults": {
        "branch": "main"
      }
    }
  },
  "job": {
    "build": {
      "input": ["name", "prompt"],
      "step": [
        {
          "name": "init",
          "run": "git worktree add worktrees/${name} -b feature/${name}",
          "on_done": "plan"
        },
        {
          "name": "plan",
          "run": { "agent": "planner" },
          "on_done": "execute"
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
    assert!(cmd.run.is_job());
    assert_eq!(cmd.run.job_name(), Some("build"));
    assert_eq!(cmd.args.positional.len(), 2);
    assert_eq!(cmd.args.positional[0].name, "name");
    assert_eq!(cmd.args.positional[1].name, "prompt");
    assert_eq!(cmd.defaults.get("branch"), Some(&"main".to_string()));

    // Jobs
    assert!(runbook.jobs.contains_key("build"));
    let job = &runbook.jobs["build"];
    assert_eq!(job.vars, vec!["name", "prompt"]);
    assert_eq!(job.steps.len(), 5);

    assert_eq!(job.steps[0].name, "init");
    assert!(job.steps[0].run.is_shell());

    assert_eq!(job.steps[1].name, "plan");
    assert!(job.steps[1].run.is_agent());
    assert_eq!(job.steps[1].agent_name(), Some("planner"));

    assert_eq!(job.steps[2].name, "execute");
    assert_eq!(
        job.steps[2].on_done.as_ref().map(|t| t.step_name()),
        Some("done")
    );
    assert_eq!(
        job.steps[2].on_fail.as_ref().map(|t| t.step_name()),
        Some("failed")
    );

    assert_eq!(job.steps[3].name, "done");
    assert_eq!(job.steps[4].name, "failed");

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
    assert!(runbook.jobs.is_empty());
}

// ============================================================================
// HCL Format Tests
// ============================================================================

const SAMPLE_HCL_RUNBOOK: &str = r#"
command "build" {
  args = "<name> <prompt>"
  run  = { job = "build" }

  defaults = {
    branch = "main"
  }
}

job "build" {
  vars  = ["name", "prompt"]

  step "init" {
    run     = "git worktree add worktrees/${name} -b feature/${name}"
    on_done = "plan"
  }

  step "plan" {
    run     = { agent = "planner" }
    on_done = "execute"
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
    assert!(cmd.run.is_job());
    assert_eq!(cmd.run.job_name(), Some("build"));
    assert_eq!(cmd.args.positional.len(), 2);
    assert_eq!(cmd.args.positional[0].name, "name");
    assert_eq!(cmd.args.positional[1].name, "prompt");
    assert_eq!(cmd.defaults.get("branch"), Some(&"main".to_string()));

    // Jobs
    assert!(runbook.jobs.contains_key("build"));
    let job = &runbook.jobs["build"];
    assert_eq!(job.vars, vec!["name", "prompt"]);
    assert_eq!(job.steps.len(), 5);

    // Steps get name from block label
    assert_eq!(job.steps[0].name, "init");
    assert!(job.steps[0].run.is_shell());

    assert_eq!(job.steps[1].name, "plan");
    assert!(job.steps[1].run.is_agent());
    assert_eq!(job.steps[1].agent_name(), Some("planner"));

    assert_eq!(job.steps[2].name, "execute");
    assert_eq!(
        job.steps[2].on_done.as_ref().map(|t| t.step_name()),
        Some("done")
    );
    assert_eq!(
        job.steps[2].on_fail.as_ref().map(|t| t.step_name()),
        Some("failed")
    );

    assert_eq!(job.steps[3].name, "done");
    assert_eq!(job.steps[4].name, "failed");

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
    assert!(runbook.jobs.is_empty());
}

#[test]
fn parse_hcl_step_names_from_block_labels() {
    let hcl = r#"
job "deploy" {
  vars  = ["env"]

  step "build" {
    run     = "make build"
    on_done = "test"
  }

  step "test" {
    run     = "make test"
    on_done = "deploy"
  }

  step "deploy" {
    run = "make deploy"
  }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let job = &runbook.jobs["deploy"];
    assert_eq!(job.steps.len(), 3);
    assert_eq!(job.steps[0].name, "build");
    assert_eq!(job.steps[1].name, "test");
    assert_eq!(
        job.steps[1].on_done.as_ref().map(|t| t.step_name()),
        Some("deploy")
    );
    assert_eq!(job.steps[2].name, "deploy");
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

// =============================================================================
// Session Config Validation Tests
// =============================================================================

#[test]
fn session_config_hcl_parses_with_color() {
    let hcl = r#"
agent "mayor" {
  run = "claude"

  session "tmux" {
    color = "cyan"
    title = "mayor"
  }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let agent = runbook.get_agent("mayor").unwrap();
    let tmux = agent.session.get("tmux").unwrap();
    assert_eq!(tmux.color.as_deref(), Some("cyan"));
    assert_eq!(tmux.title.as_deref(), Some("mayor"));
}

#[test]
fn session_config_hcl_parses_with_status() {
    let hcl = r#"
agent "mayor" {
  run = "claude"

  session "tmux" {
    color = "green"
    status {
      left  = "myproject merge/check"
      right = "custom-id"
    }
  }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let agent = runbook.get_agent("mayor").unwrap();
    let tmux = agent.session.get("tmux").unwrap();
    assert_eq!(tmux.color.as_deref(), Some("green"));
    let status = tmux.status.as_ref().unwrap();
    assert_eq!(status.left.as_deref(), Some("myproject merge/check"));
    assert_eq!(status.right.as_deref(), Some("custom-id"));
}

#[test]
fn session_config_hcl_no_session_block() {
    let hcl = r#"
agent "worker" {
  run = "claude"
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let agent = runbook.get_agent("worker").unwrap();
    assert!(agent.session.is_empty());
}

#[test]
fn session_config_hcl_unknown_provider() {
    let hcl = r#"
agent "worker" {
  run = "claude"

  session "zellij" {
    color = "red"
  }
}
"#;
    // Unknown providers parse without error
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let agent = runbook.get_agent("worker").unwrap();
    assert!(agent.session.contains_key("zellij"));
}

#[test]
fn session_config_rejects_invalid_color() {
    let hcl = r#"
agent "worker" {
  run = "claude"

  session "tmux" {
    color = "purple"
  }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("unknown color 'purple'"),
        "error should mention invalid color: {}",
        msg
    );
}

#[test]
fn session_config_accepts_all_valid_colors() {
    for color in &["red", "green", "blue", "cyan", "magenta", "yellow", "white"] {
        let hcl = format!(
            r#"
agent "worker" {{
  run = "claude"

  session "tmux" {{
    color = "{}"
  }}
}}
"#,
            color
        );
        let result = parse_runbook_with_format(&hcl, Format::Hcl);
        assert!(
            result.is_ok(),
            "color '{}' should be valid, got: {:?}",
            color,
            result.err()
        );
    }
}

#[test]
fn session_config_toml_roundtrip() {
    let toml = r#"
[agent.worker]
run = "claude"

[agent.worker.session.tmux]
color = "blue"
title = "my-worker"

[agent.worker.session.tmux.status]
left = "project build/execute"
right = "abc12345"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let agent = runbook.get_agent("worker").unwrap();
    let tmux = agent.session.get("tmux").unwrap();
    assert_eq!(tmux.color.as_deref(), Some("blue"));
    assert_eq!(tmux.title.as_deref(), Some("my-worker"));
    let status = tmux.status.as_ref().unwrap();
    assert_eq!(status.left.as_deref(), Some("project build/execute"));
    assert_eq!(status.right.as_deref(), Some("abc12345"));
}

// ============================================================================
// Epic Runbook Tests
// ============================================================================

const EPIC_HCL_RUNBOOK: &str = r#"
command "epic" {
  args = "<name> <instructions> [--blocked-by <ids>]"
  run  = { job = "epic" }

  defaults = {
    blocked-by = ""
  }
}

job "epic" {
  name      = "${var.name}"
  vars      = ["name", "instructions", "blocked-by"]
  workspace = "ephemeral"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "feature/${var.name}-${workspace.nonce}"
    title  = "feat(${var.name}): ${var.instructions}"
  }

  notify {
    on_start = "Epic started: ${var.name}"
    on_done  = "Epic landed: ${var.name}"
    on_fail  = "Epic failed: ${var.name}"
  }

  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" HEAD
    SHELL
    on_done = { step = "decompose" }
  }

  step "decompose" {
    run     = { agent = "decompose" }
    on_done = { step = "build" }
  }

  step "build" {
    run     = { agent = "epic-builder" }
    on_done = { step = "submit" }
  }

  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      test "$(git rev-list --count HEAD ^origin/main)" -gt 0 || { echo "No changes to submit" >&2; exit 1; }
      git -C "${local.repo}" push origin "${local.branch}"
      oj queue push merges --var branch="${local.branch}" --var title="${local.title}"
    SHELL
    on_done = { step = "cleanup" }
  }

  step "cleanup" {
    run = "git -C \"${local.repo}\" worktree remove --force \"${workspace.root}\" 2>/dev/null || true"
  }
}

agent "decompose" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = { action = "gate", run = "test -s .epic-root-id" }
  on_dead  = "fail"

  prime = [
    "wok prime",
    "echo '## Ready Issues'",
    "wok ready",
    "echo '## Project Instructions'",
    "cat CLAUDE.md 2>/dev/null || true",
  ]

  prompt = "Decompose the epic into tasks."
}

agent "epic-builder" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = { action = "gate", run = "root_id=$(cat .epic-root-id) && ! wok tree \"$root_id\" | grep -qE '(todo|doing)'", attempts = "forever" }
  on_dead  = { action = "resume", append = true, message = "Continue working on the epic." }

  prime = [
    "wok prime $(cat .epic-root-id)",
    "echo '## Epic Tree'",
    "wok tree $(cat .epic-root-id)",
    "echo '## Root Issue'",
    "wok show $(cat .epic-root-id)",
  ]

  prompt = "Work through the epic tasks."
}
"#;

use crate::agent::{AgentAction, Attempts, PrimeDef};
use crate::job::{WorkspaceConfig, WorkspaceType};

#[test]
fn parse_epic_hcl_command() {
    let runbook = parse_runbook_with_format(EPIC_HCL_RUNBOOK, Format::Hcl).unwrap();

    // Command
    let cmd = &runbook.commands["epic"];
    assert!(cmd.run.is_job());
    assert_eq!(cmd.run.job_name(), Some("epic"));
    assert_eq!(cmd.args.positional.len(), 2);
    assert_eq!(cmd.args.positional[0].name, "name");
    assert_eq!(cmd.args.positional[1].name, "instructions");
    assert_eq!(cmd.args.options.len(), 1);
    assert_eq!(cmd.args.options[0].name, "blocked-by");
    assert_eq!(cmd.defaults.get("blocked-by"), Some(&String::new()));
}

#[test]
fn parse_epic_hcl_job() {
    let runbook = parse_runbook_with_format(EPIC_HCL_RUNBOOK, Format::Hcl).unwrap();

    let job = &runbook.jobs["epic"];
    assert_eq!(job.name.as_deref(), Some("${var.name}"));
    assert_eq!(job.vars, vec!["name", "instructions", "blocked-by"]);
    assert_eq!(
        job.workspace,
        Some(WorkspaceConfig::Simple(WorkspaceType::Folder))
    );
    assert_eq!(job.steps.len(), 5);

    // Step names and transitions
    assert_eq!(job.steps[0].name, "init");
    assert!(job.steps[0].run.is_shell());
    assert_eq!(
        job.steps[0].on_done.as_ref().map(|t| t.step_name()),
        Some("decompose")
    );

    assert_eq!(job.steps[1].name, "decompose");
    assert!(job.steps[1].run.is_agent());
    assert_eq!(job.steps[1].agent_name(), Some("decompose"));
    assert_eq!(
        job.steps[1].on_done.as_ref().map(|t| t.step_name()),
        Some("build")
    );

    assert_eq!(job.steps[2].name, "build");
    assert!(job.steps[2].run.is_agent());
    assert_eq!(job.steps[2].agent_name(), Some("epic-builder"));
    assert_eq!(
        job.steps[2].on_done.as_ref().map(|t| t.step_name()),
        Some("submit")
    );

    assert_eq!(job.steps[3].name, "submit");
    assert!(job.steps[3].run.is_shell());
    assert_eq!(
        job.steps[3].on_done.as_ref().map(|t| t.step_name()),
        Some("cleanup")
    );

    assert_eq!(job.steps[4].name, "cleanup");
    assert!(job.steps[4].run.is_shell());
    assert!(job.steps[4].on_done.is_none()); // terminal step

    // Locals
    assert!(job.locals.contains_key("repo"));
    assert!(job.locals.contains_key("branch"));
    assert!(job.locals.contains_key("title"));

    // Notify
    assert!(job.notify.on_start.is_some());
    assert!(job.notify.on_done.is_some());
    assert!(job.notify.on_fail.is_some());
}

#[test]
fn parse_epic_hcl_decompose_agent() {
    let runbook = parse_runbook_with_format(EPIC_HCL_RUNBOOK, Format::Hcl).unwrap();

    let agent = runbook.get_agent("decompose").unwrap();
    assert!(agent.run.contains("claude"));
    assert!(agent.run.contains("--disallowed-tools"));
    assert!(agent.prompt.is_some());

    // on_idle = gate with shell check
    assert_eq!(agent.on_idle.action(), &AgentAction::Gate);
    assert_eq!(agent.on_idle.run(), Some("test -s .epic-root-id"));

    // on_dead = fail
    assert_eq!(agent.on_dead.action(), &AgentAction::Fail);

    // prime = array of 5 commands
    match &agent.prime {
        Some(PrimeDef::Commands(cmds)) => assert_eq!(cmds.len(), 5),
        other => panic!("expected PrimeDef::Commands, got {:?}", other),
    }
}

#[test]
fn parse_epic_hcl_builder_agent() {
    let runbook = parse_runbook_with_format(EPIC_HCL_RUNBOOK, Format::Hcl).unwrap();

    let agent = runbook.get_agent("epic-builder").unwrap();
    assert!(agent.run.contains("claude"));
    assert!(agent.run.contains("--disallowed-tools"));
    assert!(agent.prompt.is_some());

    // on_idle = gate with attempts = forever
    assert_eq!(agent.on_idle.action(), &AgentAction::Gate);
    assert!(agent.on_idle.run().is_some());
    assert_eq!(agent.on_idle.attempts(), Attempts::Forever);

    // on_dead = recover with append
    assert_eq!(agent.on_dead.action(), &AgentAction::Resume);
    assert!(agent.on_dead.append());
    assert!(agent.on_dead.message().is_some());

    // prime = array of 5 commands
    match &agent.prime {
        Some(PrimeDef::Commands(cmds)) => assert_eq!(cmds.len(), 5),
        other => panic!("expected PrimeDef::Commands, got {:?}", other),
    }
}
