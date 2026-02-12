// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Agent configuration tests: command recognition, prompt config, session config.

use oj_runbook::{parse_runbook, ParseError};

#[test]
fn unrecognized_agent_command() {
    let err = parse_runbook("[agent.test]\nrun = \"unknown-tool -p 'do something'\"").unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    super::assert_err_contains(&err, &["unrecognized", "unknown-tool"]);
}

#[test]
fn claude_command() {
    assert!(parse_runbook("[agent.p]\nrun = \"claude --print 'Plan'\"").is_ok());
}

#[test]
fn claudeless_command() {
    assert!(parse_runbook("[agent.r]\nrun = \"claudeless --scenario 'Run'\"").is_ok());
}

#[test]
fn absolute_path_command() {
    assert!(parse_runbook("[agent.p]\nrun = \"/usr/local/bin/claude --print 'Plan'\"").is_ok());
}

#[test]
fn unrecognized_absolute_path() {
    let err = parse_runbook("[agent.test]\nrun = \"/usr/bin/codex --help\"").unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    super::assert_err_contains(&err, &["codex"]);
}

#[test]
fn hcl_agent_validation() {
    assert!(super::parse_hcl("agent \"p\" {\n  run = \"claude --print 'Plan'\"\n}")
        .agents
        .contains_key("p"));
}

#[test]
fn hcl_unrecognized_agent_command() {
    super::assert_hcl_err("agent \"t\" {\n  run = \"unknown-tool -p 'do'\"\n}", &["unrecognized"]);
}

#[test]
fn prompt_field_no_inline() {
    let toml = "[agent.plan]\nrun = \"claude --dangerously-skip-permissions\"\nprompt = \"Plan the feature\"";
    assert!(parse_runbook(toml).is_ok());
}

#[test]
fn prompt_file_no_inline() {
    let toml = "[agent.plan]\nrun = \"claude --dangerously-skip-permissions\"\nprompt_file = \"prompts/plan.md\"";
    assert!(parse_runbook(toml).is_ok());
}

#[test]
fn prompt_field_with_positional_rejected() {
    let err = parse_runbook(
        "[agent.plan]\nrun = \"claude --print \\\"${prompt}\\\"\"\nprompt = \"Plan\"",
    )
    .unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    super::assert_err_contains(&err, &["positional"]);
}

#[test]
fn no_prompt_no_reference() {
    assert!(parse_runbook("[agent.plan]\nrun = \"claude --dangerously-skip-permissions\"").is_ok());
}

#[test]
fn prompt_reference_without_field() {
    // ${prompt} in run without a prompt field is valid — the value comes from job input
    assert!(parse_runbook("[agent.plan]\nrun = \"claude -p \\\"${prompt}\\\"\"").is_ok());
}

#[test]
fn session_id_rejected() {
    let err = parse_runbook("[agent.plan]\nrun = \"claude --session-id abc123\"").unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    super::assert_err_contains(&err, &["session-id", "automatically"]);
}

#[test]
fn session_id_equals_rejected() {
    let err = parse_runbook("[agent.plan]\nrun = \"claude --session-id=abc123\"").unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    super::assert_err_contains(&err, &["session-id"]);
}
