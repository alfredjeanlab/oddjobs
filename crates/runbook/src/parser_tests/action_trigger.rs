// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Action-trigger compatibility tests

use crate::{parse_runbook, parse_runbook_with_format, Format};

#[test]
fn on_idle_rejects_resume() {
    let toml = r#"
[agent.test]
run = "claude"
on_idle = "resume"
"#;
    let err = parse_runbook(toml).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("resume"),
        "error should mention 'resume': {}",
        msg
    );
    assert!(
        msg.contains("on_idle"),
        "error should mention 'on_idle': {}",
        msg
    );
    assert!(
        msg.contains("still running"),
        "error should mention 'still running': {}",
        msg
    );
}

#[test]
fn on_dead_rejects_nudge() {
    let toml = r#"
[agent.test]
run = "claude"
on_dead = "nudge"
"#;
    let err = parse_runbook(toml).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("nudge"),
        "error should mention 'nudge': {}",
        msg
    );
    assert!(
        msg.contains("on_dead"),
        "error should mention 'on_dead': {}",
        msg
    );
    assert!(
        msg.contains("exited"),
        "error should mention 'exited': {}",
        msg
    );
}

#[test]
fn on_error_rejects_nudge() {
    let toml = r#"
[agent.test]
run = "claude"
on_error = "nudge"
"#;
    let err = parse_runbook(toml).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("nudge"),
        "error should mention 'nudge': {}",
        msg
    );
    assert!(
        msg.contains("on_error"),
        "error should mention 'on_error': {}",
        msg
    );
}

#[test]
fn on_error_accepts_resume() {
    // Resume is now valid for on_error (rate limit recovery, transient errors)
    let toml = r#"
[agent.test]
run = "claude"
on_error = "resume"
"#;
    assert!(
        parse_runbook(toml).is_ok(),
        "on_error should accept 'resume'"
    );
}

#[test]
fn on_error_rejects_done() {
    let toml = r#"
[agent.test]
run = "claude"
on_error = "done"
"#;
    let err = parse_runbook(toml).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("done"), "error should mention 'done': {}", msg);
    assert!(
        msg.contains("on_error"),
        "error should mention 'on_error': {}",
        msg
    );
}

#[test]
fn valid_on_idle_actions() {
    for action in ["nudge", "done", "escalate", "fail", "gate"] {
        let toml = format!(
            r#"
[agent.test]
run = "claude"
on_idle = "{}"
"#,
            action
        );
        assert!(
            parse_runbook(&toml).is_ok(),
            "on_idle should accept '{}'",
            action
        );
    }
}

#[test]
fn valid_on_dead_actions() {
    for action in ["done", "resume", "escalate", "fail", "gate"] {
        let toml = format!(
            r#"
[agent.test]
run = "claude"
on_dead = "{}"
"#,
            action
        );
        assert!(
            parse_runbook(&toml).is_ok(),
            "on_dead should accept '{}'",
            action
        );
    }
}

#[test]
fn valid_on_error_actions() {
    for action in ["fail", "resume", "escalate", "gate"] {
        let toml = format!(
            r#"
[agent.test]
run = "claude"
on_error = "{}"
"#,
            action
        );
        assert!(
            parse_runbook(&toml).is_ok(),
            "on_error should accept '{}'",
            action
        );
    }
}

#[test]
fn on_idle_gate_valid() {
    let toml = r#"
[agent.test]
run = "claude"
on_idle = { action = "gate", run = "test -f output.txt" }
"#;
    assert!(parse_runbook(toml).is_ok());
}

#[test]
fn on_dead_gate_valid() {
    let toml = r#"
[agent.test]
run = "claude"
on_dead = { action = "gate", run = "test -f output.txt" }
"#;
    assert!(parse_runbook(toml).is_ok());
}

#[test]
fn on_error_gate_valid() {
    let toml = r#"
[agent.test]
run = "claude"
on_error = { action = "gate", run = "test -f output.txt" }
"#;
    assert!(parse_runbook(toml).is_ok());
}

#[test]
fn on_error_per_type_validates_all_actions() {
    let toml = r#"
[agent.test]
run = "claude"
[[agent.test.on_error]]
match = "rate_limited"
action = "nudge"
"#;
    let err = parse_runbook(toml).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("nudge"),
        "error should mention 'nudge': {}",
        msg
    );
    assert!(
        msg.contains("on_error"),
        "error should mention 'on_error': {}",
        msg
    );
}

#[test]
fn on_error_per_type_valid_actions() {
    let toml = r#"
[agent.test]
run = "claude"
[[agent.test.on_error]]
match = "rate_limited"
action = "escalate"

[[agent.test.on_error]]
match = "unauthorized"
action = "fail"
"#;
    assert!(parse_runbook(toml).is_ok());
}

#[test]
fn hcl_on_idle_rejects_resume() {
    let hcl = r#"
agent "test" {
  run = "claude"
  on_idle = "resume"
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("resume"),
        "error should mention 'resume': {}",
        msg
    );
    assert!(
        msg.contains("on_idle"),
        "error should mention 'on_idle': {}",
        msg
    );
}

#[test]
fn json_on_dead_rejects_nudge() {
    let json = r#"
{
  "agent": {
    "test": {
      "run": "claude",
      "on_dead": "nudge"
    }
  }
}
"#;
    let err = parse_runbook_with_format(json, Format::Json).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("nudge"),
        "error should mention 'nudge': {}",
        msg
    );
    assert!(
        msg.contains("on_dead"),
        "error should mention 'on_dead': {}",
        msg
    );
}
