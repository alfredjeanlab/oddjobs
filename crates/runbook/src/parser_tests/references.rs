// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::{parse_runbook, parse_runbook_with_format, Format, ParseError};

// ============================================================================
// Phase 1: Step Reference Validation
// ============================================================================

#[test]
fn error_step_on_done_references_unknown_step() {
    let toml = r#"
[job.test]
[[job.test.step]]
name = "build"
run = "echo build"
on_done = "nonexistent"

[[job.test.step]]
name = "deploy"
run = "echo deploy"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown step 'nonexistent'"),
        "expected unknown step error, got: {msg}"
    );
    assert!(
        msg.contains("step[0](build).on_done"),
        "expected step location, got: {msg}"
    );
}

#[test]
fn error_step_on_fail_references_unknown_step() {
    let toml = r#"
[job.test]
[[job.test.step]]
name = "build"
run = "echo build"
on_fail = "nonexistent"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown step 'nonexistent'"),
        "got: {msg}"
    );
    assert!(msg.contains("on_fail"), "got: {msg}");
}

#[test]
fn error_step_on_cancel_references_unknown_step() {
    let toml = r#"
[job.test]
[[job.test.step]]
name = "build"
run = "echo build"
on_cancel = "nonexistent"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown step 'nonexistent'"),
        "got: {msg}"
    );
    assert!(msg.contains("on_cancel"), "got: {msg}");
}

#[test]
fn error_job_on_done_references_unknown_step() {
    let hcl = r#"
job "test" {
  on_done = "nonexistent"

  step "build" {
    run = "echo build"
  }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown step 'nonexistent'"),
        "got: {msg}"
    );
    assert!(
        msg.contains("job.test.on_done"),
        "expected job-level location, got: {msg}"
    );
}

#[test]
fn error_job_on_fail_references_unknown_step() {
    let hcl = r#"
job "test" {
  on_fail = "nonexistent"

  step "build" {
    run = "echo build"
  }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown step 'nonexistent'"),
        "got: {msg}"
    );
    assert!(msg.contains("job.test.on_fail"), "got: {msg}");
}

#[test]
fn error_job_on_cancel_references_unknown_step() {
    let hcl = r#"
job "test" {
  on_cancel = "nonexistent"

  step "build" {
    run = "echo build"
  }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown step 'nonexistent'"),
        "got: {msg}"
    );
    assert!(msg.contains("job.test.on_cancel"), "got: {msg}");
}

#[test]
fn valid_step_references_succeed() {
    let hcl = r#"
job "deploy" {
  on_fail = "cleanup"

  step "build" {
    run = "make build"
    on_done = "test"
  }

  step "test" {
    run = "make test"
    on_done = "release"
    on_fail = "cleanup"
  }

  step "release" {
    run = "make release"
  }

  step "cleanup" {
    run = "make clean"
  }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    assert_eq!(runbook.jobs["deploy"].steps.len(), 4);
}

// ============================================================================
// Phase 2: Agent and Job Reference Validation
// ============================================================================

#[test]
fn error_step_references_unknown_agent() {
    let hcl = r#"
job "test" {
  step "work" {
    run = { agent = "ghost" }
  }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown agent 'ghost'"),
        "got: {msg}"
    );
    assert!(msg.contains("step[0](work).run"), "got: {msg}");
}

#[test]
fn error_step_references_unknown_job() {
    let hcl = r#"
job "test" {
  step "work" {
    run = { job = "nonexistent" }
  }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown job 'nonexistent'"),
        "got: {msg}"
    );
}

#[test]
fn error_command_references_unknown_agent() {
    let toml = r#"
[command.test]
run = { agent = "ghost" }
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown agent 'ghost'"),
        "got: {msg}"
    );
    assert!(msg.contains("command.test.run"), "got: {msg}");
}

#[test]
fn error_command_references_unknown_job() {
    let toml = r#"
[command.test]
run = { job = "ghost" }
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("references unknown job 'ghost'"),
        "got: {msg}"
    );
    assert!(msg.contains("command.test.run"), "got: {msg}");
}

#[test]
fn valid_agent_reference_in_step_succeeds() {
    let hcl = r#"
agent "planner" {
  run = "claude"
}

job "test" {
  step "work" {
    run = { agent = "planner" }
  }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    assert!(runbook.agents.contains_key("planner"));
    assert_eq!(
        runbook.jobs["test"].steps[0].agent_name(),
        Some("planner")
    );
}

#[test]
fn valid_job_reference_in_command_succeeds() {
    let toml = r#"
[command.build]
run = { job = "build" }

[job.build]
[[job.build.step]]
name = "run"
run = "echo build"
"#;
    let runbook = parse_runbook(toml).unwrap();
    assert!(runbook.commands.contains_key("build"));
    assert!(runbook.jobs.contains_key("build"));
}

// ============================================================================
// Phase 3: Duplicate Step Name Detection
// ============================================================================

#[test]
fn error_duplicate_step_names_in_job() {
    let toml = r#"
[job.test]
[[job.test.step]]
name = "deploy"
run = "echo first"

[[job.test.step]]
name = "deploy"
run = "echo second"
"#;
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    let msg = err.to_string();
    assert!(msg.contains("duplicate step name 'deploy'"), "got: {msg}");
}

#[test]
fn error_duplicate_step_names_hcl() {
    // HCL duplicate labeled blocks cause an HCL parse error before our
    // validation runs. This verifies that duplicate step names in HCL
    // are still rejected (just at the serde/HCL layer).
    let hcl = r#"
job "test" {
  step "build" {
    run = "echo first"
  }
  step "build" {
    run = "echo second"
  }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    // HCL parser produces an error for duplicate labeled blocks
    assert!(
        matches!(err, ParseError::Hcl(_)),
        "expected HCL parse error, got: {err}"
    );
}

#[test]
fn same_step_name_in_different_jobs_is_ok() {
    let hcl = r#"
job "a" {
  step "build" {
    run = "echo a"
  }
}

job "b" {
  step "build" {
    run = "echo b"
  }
}
"#;
    let runbook = parse_runbook_with_format(hcl, Format::Hcl).unwrap();
    assert_eq!(runbook.jobs.len(), 2);
}

// ============================================================================
// Phase 5: Unreachable Step Errors
// ============================================================================

#[test]
fn unreachable_step_is_rejected() {
    // The second step "orphan" is not referenced by any transition
    let hcl = r#"
job "test" {
  step "start" {
    run = "echo start"
    on_done = "finish"
  }

  step "orphan" {
    run = "echo orphan"
  }

  step "finish" {
    run = "echo finish"
  }
}
"#;
    let err = parse_runbook_with_format(hcl, Format::Hcl).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unreachable"),
        "error should mention unreachable: {msg}"
    );
    assert!(
        msg.contains("orphan"),
        "error should mention the step name: {msg}"
    );
}

#[test]
fn reachable_steps_parse_ok() {
    // All steps are referenced — should parse successfully
    let hcl = r#"
job "test" {
  step "start" {
    run = "echo start"
    on_done = "middle"
  }

  step "middle" {
    run = "echo middle"
    on_done = "finish"
  }

  step "finish" {
    run = "echo finish"
  }
}
"#;
    parse_runbook_with_format(hcl, Format::Hcl).unwrap();
}
