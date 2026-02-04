// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use clap::Parser;
use oj_daemon::protocol::DecisionSummary;

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

fn make_decision(id: &str, namespace: &str, pipeline: &str) -> DecisionSummary {
    DecisionSummary {
        id: id.to_string(),
        pipeline_id: "pipe-1234567890".to_string(),
        pipeline_name: pipeline.to_string(),
        source: "agent".to_string(),
        summary: "Should we proceed?".to_string(),
        created_at_ms: 0,
        namespace: namespace.to_string(),
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
    assert!(lines[0].contains("PIPELINE"));
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
