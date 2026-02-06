// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Reference validation: step refs, agent refs, job refs, duplicate steps, unreachable steps.

use oj_runbook::{parse_runbook, ParseError};

// ============================================================================
// Step Reference Validation
// ============================================================================

#[yare::parameterized(
    on_done   = { "on_done" },
    on_fail   = { "on_fail" },
    on_cancel = { "on_cancel" },
)]
fn error_step_references_unknown_step(trigger: &str) {
    let toml = format!(
        "[job.test]\n[[job.test.step]]\nname = \"build\"\nrun = \"echo build\"\n{trigger} = \"nonexistent\""
    );
    let err = parse_runbook(&toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    crate::assert_err_contains(&err, &["references unknown step 'nonexistent'", trigger]);
}

#[yare::parameterized(
    on_done   = { "on_done" },
    on_fail   = { "on_fail" },
    on_cancel = { "on_cancel" },
)]
fn error_job_references_unknown_step(trigger: &str) {
    let hcl = format!(
        "job \"test\" {{\n  {trigger} = \"nonexistent\"\n  step \"build\" {{ run = \"echo build\" }}\n}}"
    );
    crate::assert_hcl_err(&hcl, &["references unknown step 'nonexistent'", trigger]);
}

#[test]
fn valid_step_references() {
    let hcl = r#"
job "deploy" {
  on_fail = "cleanup"

  step "build" {
    run     = "make build"
    on_done = "test"
  }

  step "test" {
    run     = "make test"
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
    assert_eq!(super::parse_hcl(hcl).jobs["deploy"].steps.len(), 4);
}

// ============================================================================
// Agent and Job Reference Validation
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
    super::assert_hcl_err(
        hcl,
        &["references unknown agent 'ghost'", "step[0](work).run"],
    );
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
    super::assert_hcl_err(hcl, &["references unknown job 'nonexistent'"]);
}

#[test]
fn error_command_references_unknown_agent() {
    super::assert_toml_err(
        "[command.test]\nrun = { agent = \"ghost\" }",
        &["references unknown agent 'ghost'", "command.test.run"],
    );
}

#[test]
fn error_command_references_unknown_job() {
    super::assert_toml_err(
        "[command.test]\nrun = { job = \"ghost\" }",
        &["references unknown job 'ghost'", "command.test.run"],
    );
}

#[test]
fn valid_agent_reference_in_step() {
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
    assert_eq!(
        super::parse_hcl(hcl).jobs["test"].steps[0].agent_name(),
        Some("planner")
    );
}

#[test]
fn valid_job_reference_in_command() {
    let toml = "[command.build]\nrun = { job = \"build\" }\n[job.build]\n[[job.build.step]]\nname = \"run\"\nrun = \"echo build\"";
    assert!(parse_runbook(toml).is_ok());
}

// ============================================================================
// Duplicate Step Names
// ============================================================================

#[test]
fn error_duplicate_step_names_in_job() {
    let toml = "[job.test]\n[[job.test.step]]\nname = \"deploy\"\nrun = \"echo first\"\n\n[[job.test.step]]\nname = \"deploy\"\nrun = \"echo second\"";
    let err = parse_runbook(toml).unwrap_err();
    assert!(matches!(err, ParseError::InvalidFormat { .. }));
    super::assert_err_contains(&err, &["duplicate step name 'deploy'"]);
}

#[test]
fn error_duplicate_step_names_hcl() {
    let hcl = r#"
job "test" {
  step "build" { run = "echo first" }
  step "build" { run = "echo second" }
}
"#;
    let err = oj_runbook::parse_runbook_with_format(hcl, oj_runbook::Format::Hcl).unwrap_err();
    assert!(matches!(err, oj_runbook::ParseError::Hcl(_)));
}

#[test]
fn same_step_name_in_different_jobs() {
    let hcl = r#"
job "a" {
  step "build" { run = "echo a" }
}

job "b" {
  step "build" { run = "echo b" }
}
"#;
    assert_eq!(super::parse_hcl(hcl).jobs.len(), 2);
}

// ============================================================================
// Unreachable Steps
// ============================================================================

#[test]
fn unreachable_step_is_rejected() {
    let hcl = r#"
job "test" {
  step "start" {
    run     = "echo start"
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
    super::assert_hcl_err(hcl, &["unreachable", "orphan"]);
}

#[test]
fn reachable_steps_parse_ok() {
    let hcl = r#"
job "test" {
  step "start" {
    run     = "echo start"
    on_done = "middle"
  }

  step "middle" {
    run     = "echo middle"
    on_done = "finish"
  }

  step "finish" {
    run = "echo finish"
  }
}
"#;
    super::parse_hcl(hcl);
}
