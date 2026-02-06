// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Action-trigger compatibility tests.

use oj_runbook::{parse_runbook, parse_runbook_with_format, Format};

// ============================================================================
// Rejection Tests
// ============================================================================

#[test]
fn on_idle_rejects_resume() {
    super::assert_toml_err(
        "[agent.test]\nrun = \"claude\"\non_idle = \"resume\"",
        &["resume", "on_idle", "still running"],
    );
}

#[test]
fn on_dead_rejects_nudge() {
    super::assert_toml_err(
        "[agent.test]\nrun = \"claude\"\non_dead = \"nudge\"",
        &["nudge", "on_dead", "exited"],
    );
}

#[test]
fn on_error_rejects_nudge() {
    super::assert_toml_err(
        "[agent.test]\nrun = \"claude\"\non_error = \"nudge\"",
        &["nudge", "on_error"],
    );
}

#[test]
fn on_error_rejects_done() {
    super::assert_toml_err(
        "[agent.test]\nrun = \"claude\"\non_error = \"done\"",
        &["done", "on_error"],
    );
}

#[test]
fn on_error_per_type_validates_all_actions() {
    let toml = "[agent.test]\nrun = \"claude\"\n[[agent.test.on_error]]\nmatch = \"rate_limited\"\naction = \"nudge\"";
    super::assert_toml_err(toml, &["nudge", "on_error"]);
}

// ============================================================================
// Valid Actions
// ============================================================================

#[test]
fn on_error_accepts_resume() {
    assert!(parse_runbook("[agent.test]\nrun = \"claude\"\non_error = \"resume\"").is_ok());
}

#[test]
fn valid_on_idle_actions() {
    for action in ["nudge", "done", "escalate", "fail", "gate"] {
        let toml = format!("[agent.test]\nrun = \"claude\"\non_idle = \"{action}\"");
        assert!(
            parse_runbook(&toml).is_ok(),
            "on_idle should accept '{action}'"
        );
    }
}

#[test]
fn valid_on_dead_actions() {
    for action in ["done", "resume", "escalate", "fail", "gate"] {
        let toml = format!("[agent.test]\nrun = \"claude\"\non_dead = \"{action}\"");
        assert!(
            parse_runbook(&toml).is_ok(),
            "on_dead should accept '{action}'"
        );
    }
}

#[test]
fn valid_on_error_actions() {
    for action in ["fail", "resume", "escalate", "gate"] {
        let toml = format!("[agent.test]\nrun = \"claude\"\non_error = \"{action}\"");
        assert!(
            parse_runbook(&toml).is_ok(),
            "on_error should accept '{action}'"
        );
    }
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

// ============================================================================
// Gate Actions
// ============================================================================

#[yare::parameterized(
    on_idle  = { "on_idle" },
    on_dead  = { "on_dead" },
    on_error = { "on_error" },
)]
fn gate_valid(trigger: &str) {
    let toml = format!(
        "[agent.test]\nrun = \"claude\"\n{trigger} = {{ action = \"gate\", run = \"test -f output.txt\" }}"
    );
    assert!(
        parse_runbook(&toml).is_ok(),
        "gate should be valid for {trigger}"
    );
}

// ============================================================================
// Cross-Format
// ============================================================================

#[test]
fn hcl_on_idle_rejects_resume() {
    super::assert_hcl_err(
        "agent \"test\" {\n  run = \"claude\"\n  on_idle = \"resume\"\n}",
        &["resume", "on_idle"],
    );
}

#[test]
fn json_on_dead_rejects_nudge() {
    let json = r#"{"agent":{"test":{"run":"claude","on_dead":"nudge"}}}"#;
    let err = parse_runbook_with_format(json, Format::Json).unwrap_err();
    super::assert_err_contains(&err, &["nudge", "on_dead"]);
}
