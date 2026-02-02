// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::{parse_runbook, parse_runbook_with_format, Format, ParseError, PrimeDef};

#[test]
fn parse_agent_with_prime_string() {
    let toml = r#"
[agent.worker]
run = "claude"
prime = "echo hello\ngit status"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let agent = &runbook.agents["worker"];
    assert!(matches!(agent.prime, Some(PrimeDef::Script(_))));
}

#[test]
fn parse_agent_with_prime_array() {
    let toml = r#"
[agent.worker]
run = "claude"
prime = ["echo hello", "git status --short"]
"#;
    let runbook = parse_runbook(toml).unwrap();
    let agent = &runbook.agents["worker"];
    assert!(matches!(agent.prime, Some(PrimeDef::Commands(_))));
}

#[test]
fn error_prime_array_invalid_shell() {
    let toml = r#"
[agent.worker]
run = "claude"
prime = ["echo hello", "echo 'unterminated"]
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::ShellError { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("agent.worker.prime[1]"),
        "error should mention prime location: {}",
        msg
    );
}

#[test]
fn parse_agent_with_prime_string_multiline() {
    // String form allows multi-line scripts (no per-command validation)
    let toml = r#"
[agent.worker]
run = "claude"
prime = """
echo '## Git Status'
git status --short | head -10
if [ -f PLAN.md ]; then
  cat PLAN.md
fi
"""
"#;
    let runbook = parse_runbook(toml).unwrap();
    let agent = &runbook.agents["worker"];
    assert!(matches!(agent.prime, Some(PrimeDef::Script(_))));
}

#[test]
fn parse_hcl_agent_with_prime_array() {
    let hcl = r#"
agent "worker" {
  run   = "claude"
  prime = ["echo hello", "git status --short"]
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    let agent = &runbook.agents["worker"];
    assert!(matches!(agent.prime, Some(PrimeDef::Commands(_))));
}

#[test]
fn parse_agent_without_prime() {
    let toml = r#"
[agent.worker]
run = "claude"
"#;
    let runbook = parse_runbook(toml).unwrap();
    let agent = &runbook.agents["worker"];
    assert!(agent.prime.is_none());
}
