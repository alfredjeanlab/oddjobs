// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use clap::Parser;
use oj_daemon::protocol::{DecisionDetail, DecisionOptionDetail, DecisionSummary};

use super::*;

/// Wrapper for testing DecisionCommand parsing
#[derive(Parser)]
struct TestCli {
    #[command(subcommand)]
    command: DecisionCommand,
}

#[test]
fn parse_list() {
    let cli = TestCli::parse_from(["test", "list"]);
    assert!(matches!(cli.command, DecisionCommand::List {}));
}

#[test]
fn parse_show() {
    let cli = TestCli::parse_from(["test", "show", "abc123"]);
    if let DecisionCommand::Show { id } = cli.command {
        assert_eq!(id, "abc123");
    } else {
        panic!("expected Show");
    }
}

#[test]
fn parse_review() {
    let cli = TestCli::parse_from(["test", "review"]);
    assert!(matches!(cli.command, DecisionCommand::Review {}));
}

#[test]
fn parse_resolve_with_choice() {
    let cli = TestCli::parse_from(["test", "resolve", "abc123", "2"]);
    if let DecisionCommand::Resolve {
        id,
        choice,
        message,
    } = cli.command
    {
        assert_eq!(id, "abc123");
        assert_eq!(choice, Some(2));
        assert_eq!(message, None);
    } else {
        panic!("expected Resolve");
    }
}

#[test]
fn parse_resolve_with_message() {
    let cli = TestCli::parse_from(["test", "resolve", "abc123", "-m", "looks good"]);
    if let DecisionCommand::Resolve {
        id,
        choice,
        message,
    } = cli.command
    {
        assert_eq!(id, "abc123");
        assert_eq!(choice, None);
        assert_eq!(message, Some("looks good".to_string()));
    } else {
        panic!("expected Resolve");
    }
}

#[test]
fn parse_resolve_with_choice_and_message() {
    let cli = TestCli::parse_from(["test", "resolve", "abc123", "1", "-m", "approved"]);
    if let DecisionCommand::Resolve {
        id,
        choice,
        message,
    } = cli.command
    {
        assert_eq!(id, "abc123");
        assert_eq!(choice, Some(1));
        assert_eq!(message, Some("approved".to_string()));
    } else {
        panic!("expected Resolve");
    }
}

fn make_decision(id: &str, namespace: &str, job: &str) -> DecisionSummary {
    DecisionSummary {
        id: id.to_string(),
        job_id: "pipe-1234567890".to_string(),
        job_name: job.to_string(),
        source: "agent".to_string(),
        summary: "Should we proceed?".to_string(),
        created_at_ms: 0,
        namespace: namespace.to_string(),
    }
}

fn make_detail(resolved: bool) -> DecisionDetail {
    DecisionDetail {
        id: "abcdef1234567890".to_string(),
        job_id: "pipe-1234567890".to_string(),
        job_name: "build".to_string(),
        agent_id: Some("agent-abc12345".to_string()),
        source: "agent".to_string(),
        context: "Should we deploy?".to_string(),
        options: vec![
            DecisionOptionDetail {
                number: 1,
                label: "Yes".to_string(),
                description: Some("Deploy now".to_string()),
                recommended: true,
            },
            DecisionOptionDetail {
                number: 2,
                label: "No".to_string(),
                description: None,
                recommended: false,
            },
        ],
        chosen: if resolved { Some(1) } else { None },
        message: if resolved {
            Some("approved".to_string())
        } else {
            None
        },
        created_at_ms: 0,
        resolved_at_ms: if resolved { Some(1000) } else { None },
        superseded_by: None,
        namespace: "myproject".to_string(),
    }
}

fn output_string(buf: &[u8]) -> String {
    String::from_utf8(buf.to_vec()).unwrap()
}

#[test]
fn list_uses_table_with_dynamic_widths() {
    let decisions = vec![
        make_decision("abcdef1234567890", "", "build"),
        make_decision("1234567890abcdef", "", "deploy-service"),
    ];
    let mut buf = Vec::new();
    super::format_decision_list(&mut buf, &decisions);
    let out = output_string(&buf);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("ID"));
    assert!(lines[0].contains("JOB"));
    assert!(lines[0].contains("SOURCE"));
    assert!(!lines[0].contains("PROJECT"));
    // ID should be truncated to 8 chars
    assert!(lines[1].contains("abcdef12"));
}

#[test]
fn list_with_project_column() {
    let decisions = vec![
        make_decision("abcdef1234567890", "myproject", "build"),
        make_decision("1234567890abcdef", "other", "deploy"),
    ];
    let mut buf = Vec::new();
    super::format_decision_list(&mut buf, &decisions);
    let out = output_string(&buf);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("PROJECT"));
    assert!(lines[1].contains("myproject"));
    assert!(lines[2].contains("other"));
}

// --- format_decision_detail tests ---

#[test]
fn format_decision_detail_with_hint() {
    let d = make_detail(false);
    let mut buf = Vec::new();
    super::format_decision_detail(&mut buf, &d, true);
    let out = output_string(&buf);

    assert!(out.contains("Decision:"));
    assert!(out.contains("abcdef12"));
    assert!(out.contains("Job:"));
    assert!(out.contains("build"));
    assert!(out.contains("Source:"));
    assert!(out.contains("agent"));
    assert!(out.contains("Context:"));
    assert!(out.contains("Should we deploy?"));
    assert!(out.contains("Options:"));
    assert!(out.contains("1. Yes (recommended) - Deploy now"));
    assert!(out.contains("2. No"));
    assert!(out.contains("oj decision resolve abcdef12 <number>"));
}

#[test]
fn format_decision_detail_without_hint() {
    let d = make_detail(false);
    let mut buf = Vec::new();
    super::format_decision_detail(&mut buf, &d, false);
    let out = output_string(&buf);

    assert!(out.contains("Decision:"));
    assert!(out.contains("Options:"));
    assert!(!out.contains("oj decision resolve"));
}

#[test]
fn format_decision_detail_resolved() {
    let d = make_detail(true);
    let mut buf = Vec::new();
    super::format_decision_detail(&mut buf, &d, true);
    let out = output_string(&buf);

    assert!(out.contains("Status:"));
    assert!(out.contains("completed"));
    assert!(out.contains("Chosen:"));
    assert!(out.contains("1 (Yes)"));
    assert!(out.contains("Message:"));
    assert!(out.contains("approved"));
    // Resolve hint should NOT appear for resolved decisions
    assert!(!out.contains("oj decision resolve"));
}

// --- parse_review_input tests ---

#[test]
fn review_input_pick_valid() {
    assert_eq!(parse_review_input("1", 3), ReviewAction::Pick(1));
    assert_eq!(parse_review_input("2", 3), ReviewAction::Pick(2));
    assert_eq!(parse_review_input("3", 3), ReviewAction::Pick(3));
    assert_eq!(parse_review_input(" 2 ", 3), ReviewAction::Pick(2));
}

#[test]
fn review_input_pick_out_of_range() {
    assert_eq!(parse_review_input("0", 3), ReviewAction::Invalid);
    assert_eq!(parse_review_input("4", 3), ReviewAction::Invalid);
    assert_eq!(parse_review_input("1", 0), ReviewAction::Invalid);
}

#[test]
fn review_input_skip() {
    assert_eq!(parse_review_input("", 3), ReviewAction::Skip);
    assert_eq!(parse_review_input("s", 3), ReviewAction::Skip);
    assert_eq!(parse_review_input("S", 3), ReviewAction::Skip);
    assert_eq!(parse_review_input("  ", 3), ReviewAction::Skip);
}

#[test]
fn review_input_quit() {
    assert_eq!(parse_review_input("q", 3), ReviewAction::Quit);
    assert_eq!(parse_review_input("Q", 3), ReviewAction::Quit);
    assert_eq!(parse_review_input("x", 3), ReviewAction::Quit);
    assert_eq!(parse_review_input("X", 3), ReviewAction::Quit);
}

#[test]
fn review_input_invalid() {
    assert_eq!(parse_review_input("abc", 3), ReviewAction::Invalid);
    assert_eq!(parse_review_input("-1", 3), ReviewAction::Invalid);
    assert_eq!(parse_review_input("pick", 3), ReviewAction::Invalid);
}
