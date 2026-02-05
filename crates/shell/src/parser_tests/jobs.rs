// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for jobs, logical operators, and precedence.

use super::helpers::{cmd_name, get_job, get_simple_command};
use super::macros::job_tests;
use crate::ast::{Command, LogicalOp, WordPart};
use crate::parser::{ParseError, Parser};
use crate::token::TokenKind;

// =============================================================================
// Macro-based Job Tests
// =============================================================================

job_tests! {
    macro_two_pipe: "a | b" => job_cmds: 2,
    macro_three_pipe: "a | b | c" => job_cmds: 3,
    macro_four_pipe: "a | b | c | d" => job_cmds: 4,
}

// ============================================================================
// Basic Job Tests
// ============================================================================

#[test]
fn test_simple_pipe() {
    let result = Parser::parse("cmd1 | cmd2").unwrap();
    assert_eq!(result.commands.len(), 1);

    let job = get_job(&result.commands[0]);
    assert_eq!(job.commands.len(), 2);
    assert_eq!(cmd_name(&job.commands[0]), "cmd1");
    assert_eq!(cmd_name(&job.commands[1]), "cmd2");
}

#[test]
fn test_pipe_chain() {
    let result = Parser::parse("cmd1 | cmd2 | cmd3").unwrap();
    assert_eq!(result.commands.len(), 1);

    let job = get_job(&result.commands[0]);
    assert_eq!(job.commands.len(), 3);
    assert_eq!(cmd_name(&job.commands[0]), "cmd1");
    assert_eq!(cmd_name(&job.commands[1]), "cmd2");
    assert_eq!(cmd_name(&job.commands[2]), "cmd3");
}

#[test]
fn test_pipe_with_args() {
    let result = Parser::parse("ls -la | grep foo | wc -l").unwrap();
    assert_eq!(result.commands.len(), 1);

    let job = get_job(&result.commands[0]);
    assert_eq!(job.commands.len(), 3);

    assert_eq!(cmd_name(&job.commands[0]), "ls");
    assert_eq!(job.commands[0].args.len(), 1);
    assert_eq!(
        job.commands[0].args[0].parts,
        vec![WordPart::literal("-la")]
    );

    assert_eq!(cmd_name(&job.commands[1]), "grep");
    assert_eq!(job.commands[1].args.len(), 1);

    assert_eq!(cmd_name(&job.commands[2]), "wc");
    assert_eq!(job.commands[2].args.len(), 1);
}

#[test]
fn test_variable_in_job() {
    let result = Parser::parse("$CMD | cmd2").unwrap();
    assert_eq!(result.commands.len(), 1);

    let job = get_job(&result.commands[0]);
    assert_eq!(job.commands.len(), 2);
    assert!(matches!(
        &job.commands[0].name.parts[0],
        WordPart::Variable { name, .. } if name == "CMD"
    ));
}

#[test]
fn test_pipe_span() {
    let result = Parser::parse("cat foo | grep bar").unwrap();
    let job = get_job(&result.commands[0]);

    // Span should cover from 'cat' to 'bar'
    assert_eq!(job.span.start, 0);
    assert_eq!(job.span.end, 18);
}

#[test]
fn test_pipe_at_start_error() {
    let err = Parser::parse("| cmd").unwrap_err();
    assert!(matches!(
        err,
        ParseError::UnexpectedToken {
            found: TokenKind::Pipe,
            ..
        }
    ));
}

#[test]
fn test_pipe_at_end_error() {
    let err = Parser::parse("cmd |").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedEof { .. }));
}

// ============================================================================
// Background Execution Tests
// ============================================================================

#[test]
fn test_simple_background() {
    let result = Parser::parse("cmd &").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert!(and_or.first.background);
    assert!(matches!(&and_or.first.command, Command::Simple(_)));
}

#[test]
fn test_job_background() {
    let result = Parser::parse("cmd1 | cmd2 &").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert!(and_or.first.background);

    let Command::Job(p) = &and_or.first.command else {
        panic!("Expected job");
    };
    assert_eq!(p.commands.len(), 2);
}

#[test]
fn test_background_in_sequence() {
    // a & b -> two items: (a &), then b
    // & acts as a separator similar to ; but with background
    let result = Parser::parse("a & b").unwrap();
    assert_eq!(result.commands.len(), 2);

    // First command: a with background
    let and_or1 = &result.commands[0];
    assert!(and_or1.first.background);
    let Command::Simple(a) = &and_or1.first.command else {
        panic!("Expected simple command");
    };
    assert_eq!(cmd_name(a), "a");

    // Second command: b without background
    let and_or2 = &result.commands[1];
    assert!(!and_or2.first.background);
    let Command::Simple(b) = &and_or2.first.command else {
        panic!("Expected simple command");
    };
    assert_eq!(cmd_name(b), "b");
}

#[test]
fn test_background_then_foreground() {
    let result = Parser::parse("cmd1 & ; cmd2").unwrap();
    assert_eq!(result.commands.len(), 2);

    assert!(result.commands[0].first.background);
    assert!(!result.commands[1].first.background);
}

#[test]
fn test_multiple_background() {
    let result = Parser::parse("cmd1 & ; cmd2 & ; cmd3").unwrap();
    assert_eq!(result.commands.len(), 3);

    assert!(result.commands[0].first.background);
    assert!(result.commands[1].first.background);
    assert!(!result.commands[2].first.background);
}

#[test]
fn test_background_span() {
    let result = Parser::parse("cmd &").unwrap();
    let and_or = &result.commands[0];

    // Span should cover from 'cmd' to '&'
    assert_eq!(and_or.first.span.start, 0);
    assert_eq!(and_or.first.span.end, 5);
}

#[test]
fn test_ampersand_at_start_error() {
    let err = Parser::parse("& cmd").unwrap_err();
    assert!(matches!(
        err,
        ParseError::UnexpectedToken {
            found: TokenKind::Ampersand,
            ..
        }
    ));
}

#[test]
fn test_background_with_and() {
    // a && b & -> a && (b &)
    // Only b runs in background
    let result = Parser::parse("a && b &").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    // First item: a, not backgrounded
    assert!(!and_or.first.background);

    // After &&: b, backgrounded
    assert_eq!(and_or.rest.len(), 1);
    assert!(and_or.rest[0].1.background);
}

#[test]
fn test_background_followed_by_and_error() {
    // a & && b -> error: unexpected && after background
    // After 'a &', we expect separator or end, not &&
    let err = Parser::parse("a & && b").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { .. }));
}

// ============================================================================
// Logical Operator Tests
// ============================================================================

#[test]
fn test_and_basic() {
    let result = Parser::parse("cmd1 && cmd2").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert_eq!(and_or.rest.len(), 1);
    assert_eq!(and_or.rest[0].0, LogicalOp::And);

    let Command::Simple(first) = &and_or.first.command else {
        panic!("Expected simple command");
    };
    assert_eq!(cmd_name(first), "cmd1");

    let Command::Simple(second) = &and_or.rest[0].1.command else {
        panic!("Expected simple command");
    };
    assert_eq!(cmd_name(second), "cmd2");
}

#[test]
fn test_or_basic() {
    let result = Parser::parse("cmd1 || cmd2").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert_eq!(and_or.rest.len(), 1);
    assert_eq!(and_or.rest[0].0, LogicalOp::Or);
}

#[test]
fn test_and_chain() {
    let result = Parser::parse("cmd1 && cmd2 && cmd3").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert_eq!(and_or.rest.len(), 2);
    assert_eq!(and_or.rest[0].0, LogicalOp::And);
    assert_eq!(and_or.rest[1].0, LogicalOp::And);
}

#[test]
fn test_or_chain() {
    let result = Parser::parse("cmd1 || cmd2 || cmd3").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert_eq!(and_or.rest.len(), 2);
    assert_eq!(and_or.rest[0].0, LogicalOp::Or);
    assert_eq!(and_or.rest[1].0, LogicalOp::Or);
}

#[test]
fn test_mixed_and_or() {
    // a && b || c && d -> ((a && b) || c) && d (left-associative)
    let result = Parser::parse("a && b || c && d").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert_eq!(and_or.rest.len(), 3);
    assert_eq!(and_or.rest[0].0, LogicalOp::And); // && b
    assert_eq!(and_or.rest[1].0, LogicalOp::Or); // || c
    assert_eq!(and_or.rest[2].0, LogicalOp::And); // && d
}

#[test]
fn test_and_or_span() {
    let result = Parser::parse("a && b || c").unwrap();
    let and_or = &result.commands[0];

    assert_eq!(and_or.span.start, 0);
    assert_eq!(and_or.span.end, 11);
}

#[test]
fn test_and_at_start_error() {
    let err = Parser::parse("&& cmd").unwrap_err();
    assert!(matches!(
        err,
        ParseError::UnexpectedToken {
            found: TokenKind::And,
            ..
        }
    ));
}

#[test]
fn test_or_at_start_error() {
    let err = Parser::parse("|| cmd").unwrap_err();
    assert!(matches!(
        err,
        ParseError::UnexpectedToken {
            found: TokenKind::Or,
            ..
        }
    ));
}

#[test]
fn test_and_at_end_error() {
    let err = Parser::parse("cmd &&").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedEof { .. }));
}

#[test]
fn test_or_at_end_error() {
    let err = Parser::parse("cmd ||").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedEof { .. }));
}

// ============================================================================
// Precedence Tests
// ============================================================================

#[test]
fn test_pipe_binds_tighter_than_and() {
    // a | b && c -> (a | b) && c
    let result = Parser::parse("a | b && c").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    // First item should be a job
    let Command::Job(job) = &and_or.first.command else {
        panic!("Expected job on left of AND");
    };
    assert_eq!(job.commands.len(), 2);
    assert_eq!(cmd_name(&job.commands[0]), "a");
    assert_eq!(cmd_name(&job.commands[1]), "b");

    // Second item (after &&) should be simple command
    assert_eq!(and_or.rest.len(), 1);
    assert_eq!(and_or.rest[0].0, LogicalOp::And);
    let Command::Simple(c) = &and_or.rest[0].1.command else {
        panic!("Expected simple command");
    };
    assert_eq!(cmd_name(c), "c");
}

#[test]
fn test_pipe_binds_tighter_than_or() {
    // a || b | c -> a || (b | c)
    let result = Parser::parse("a || b | c").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    // First item should be simple command
    let Command::Simple(a) = &and_or.first.command else {
        panic!("Expected simple command");
    };
    assert_eq!(cmd_name(a), "a");

    // Second item (after ||) should be a job
    assert_eq!(and_or.rest.len(), 1);
    assert_eq!(and_or.rest[0].0, LogicalOp::Or);
    let Command::Job(job) = &and_or.rest[0].1.command else {
        panic!("Expected job on right of OR");
    };
    assert_eq!(job.commands.len(), 2);
    assert_eq!(cmd_name(&job.commands[0]), "b");
    assert_eq!(cmd_name(&job.commands[1]), "c");
}

#[test]
fn test_complex_precedence() {
    // a | b && c | d || e | f
    // Expected: ((a | b) && (c | d)) || (e | f)
    let result = Parser::parse("a | b && c | d || e | f").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    // Should be: (a | b), &&, (c | d), ||, (e | f)
    assert_eq!(and_or.rest.len(), 2);

    // First: job a | b
    let Command::Job(p1) = &and_or.first.command else {
        panic!("Expected job");
    };
    assert_eq!(p1.commands.len(), 2);

    // && (c | d)
    assert_eq!(and_or.rest[0].0, LogicalOp::And);
    let Command::Job(p2) = &and_or.rest[0].1.command else {
        panic!("Expected job");
    };
    assert_eq!(p2.commands.len(), 2);

    // || (e | f)
    assert_eq!(and_or.rest[1].0, LogicalOp::Or);
    let Command::Job(p3) = &and_or.rest[1].1.command else {
        panic!("Expected job");
    };
    assert_eq!(p3.commands.len(), 2);
}

#[test]
fn test_pipe_chain_with_and() {
    // a | b | c && d | e
    // Expected: (a | b | c) && (d | e)
    let result = Parser::parse("a | b | c && d | e").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];

    // First: job a | b | c
    let Command::Job(p1) = &and_or.first.command else {
        panic!("Expected job");
    };
    assert_eq!(p1.commands.len(), 3);

    // && (d | e)
    assert_eq!(and_or.rest.len(), 1);
    assert_eq!(and_or.rest[0].0, LogicalOp::And);
    let Command::Job(p2) = &and_or.rest[0].1.command else {
        panic!("Expected job");
    };
    assert_eq!(p2.commands.len(), 2);
}

// ============================================================================
// Semicolon Separation Tests
// ============================================================================

#[test]
fn test_semicolon_separates_and_or_lists() {
    // a && b ; c && d
    let result = Parser::parse("a && b ; c && d").unwrap();
    assert_eq!(result.commands.len(), 2);

    // First list: a && b
    let and_or1 = &result.commands[0];
    assert_eq!(and_or1.rest.len(), 1);

    // Second list: c && d
    let and_or2 = &result.commands[1];
    assert_eq!(and_or2.rest.len(), 1);
}

#[test]
fn test_newline_separates_and_or_lists() {
    let result = Parser::parse("a && b\nc || d").unwrap();
    assert_eq!(result.commands.len(), 2);
}

#[test]
fn test_background_separates_sequence() {
    // a & b | c means: a runs in background, then b | c runs
    let result = Parser::parse("a & ; b | c").unwrap();
    assert_eq!(result.commands.len(), 2);

    // First is just 'a' in background
    assert!(result.commands[0].first.background);
    assert!(matches!(
        &result.commands[0].first.command,
        Command::Simple(_)
    ));

    // Second is 'b | c' job
    assert!(!result.commands[1].first.background);
    assert!(matches!(&result.commands[1].first.command, Command::Job(_)));
}

// ============================================================================
// Real-World Pattern Tests
// ============================================================================

#[test]
fn test_build_pattern() {
    // make && ./run || echo "build failed"
    let result = Parser::parse(r#"make && ./run || echo "build failed""#).unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert_eq!(and_or.rest.len(), 2);

    // make
    let Command::Simple(make) = &and_or.first.command else {
        panic!("Expected simple command");
    };
    assert_eq!(cmd_name(make), "make");

    // && ./run
    assert_eq!(and_or.rest[0].0, LogicalOp::And);

    // || echo "build failed"
    assert_eq!(and_or.rest[1].0, LogicalOp::Or);
}

#[test]
fn test_pipe_filter_pattern() {
    // ps aux | grep nginx | awk '{print $2}' | xargs kill
    let result = Parser::parse("ps aux | grep nginx | awk something | xargs kill").unwrap();
    assert_eq!(result.commands.len(), 1);

    let job = get_job(&result.commands[0]);
    assert_eq!(job.commands.len(), 4);
    assert_eq!(cmd_name(&job.commands[0]), "ps");
    assert_eq!(cmd_name(&job.commands[1]), "grep");
    assert_eq!(cmd_name(&job.commands[2]), "awk");
    assert_eq!(cmd_name(&job.commands[3]), "xargs");
}

#[test]
fn test_conditional_pipe() {
    // cat file && cat file | wc -l || echo "no file"
    // Expected: cat file && (cat file | wc -l) || echo "no file"
    let result = Parser::parse("cat file && cat file | wc -l || echo nofile").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert_eq!(and_or.rest.len(), 2);

    // First: cat file (simple command)
    let Command::Simple(cat1) = &and_or.first.command else {
        panic!("Expected simple command");
    };
    assert_eq!(cmd_name(cat1), "cat");

    // && (cat file | wc -l)
    assert_eq!(and_or.rest[0].0, LogicalOp::And);
    let Command::Job(job) = &and_or.rest[0].1.command else {
        panic!("Expected job");
    };
    assert_eq!(job.commands.len(), 2);
    assert_eq!(cmd_name(&job.commands[0]), "cat");
    assert_eq!(cmd_name(&job.commands[1]), "wc");

    // || echo nofile
    assert_eq!(and_or.rest[1].0, LogicalOp::Or);
}

#[test]
fn test_real_world_job() {
    let result = Parser::parse("ps aux | grep nginx | head -10").unwrap();
    assert_eq!(result.commands.len(), 1);

    let job = get_job(&result.commands[0]);
    assert_eq!(job.commands.len(), 3);
}

#[test]
fn test_real_world_conditional() {
    let result = Parser::parse("make && make test || echo 'Build failed'").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];
    assert_eq!(and_or.rest.len(), 2);
    assert_eq!(and_or.rest[0].0, LogicalOp::And);
    assert_eq!(and_or.rest[1].0, LogicalOp::Or);
}

#[test]
fn test_real_world_background_job() {
    let result = Parser::parse("long_running_task & ; echo 'Started'").unwrap();
    assert_eq!(result.commands.len(), 2);
    assert!(result.commands[0].first.background);
    assert!(!result.commands[1].first.background);
}

#[test]
fn test_all_operators_combined() {
    // Complex real-world-ish command
    let result = Parser::parse("cmd1 | cmd2 && cmd3 || cmd4 | cmd5 &").unwrap();
    assert_eq!(result.commands.len(), 1);

    let and_or = &result.commands[0];

    // First: cmd1 | cmd2 (job)
    assert!(matches!(&and_or.first.command, Command::Job(_)));

    // && cmd3 (simple)
    assert_eq!(and_or.rest[0].0, LogicalOp::And);
    assert!(matches!(&and_or.rest[0].1.command, Command::Simple(_)));

    // || cmd4 | cmd5 (job) with background
    assert_eq!(and_or.rest[1].0, LogicalOp::Or);
    assert!(matches!(&and_or.rest[1].1.command, Command::Job(_)));
    assert!(and_or.rest[1].1.background);
}

// ============================================================================
// Error Recovery Tests
// ============================================================================

#[test]
fn test_recovery_after_pipe_error() {
    // | ; cmd -> recovers, parses cmd
    let result = Parser::parse_with_recovery("| ; cmd");
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.commands.commands.len(), 1);

    let cmd = get_simple_command(&result.commands.commands[0]);
    assert_eq!(cmd_name(cmd), "cmd");
}

#[test]
fn test_recovery_preserves_partial_chain() {
    // cmd1 && cmd2 ; | ; cmd3
    let result = Parser::parse_with_recovery("cmd1 && cmd2 ; | ; cmd3");

    // Should parse: (cmd1 && cmd2) and cmd3, with 1 error for |
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.commands.commands.len(), 2);

    // First: cmd1 && cmd2
    let and_or1 = &result.commands.commands[0];
    assert_eq!(and_or1.rest.len(), 1);

    // Second: cmd3
    let and_or2 = &result.commands.commands[1];
    assert!(and_or2.rest.is_empty());
}

#[test]
fn test_recovery_multiple_and_or_errors() {
    let result = Parser::parse_with_recovery("| ; && ; ||");
    assert_eq!(result.errors.len(), 3);
    assert!(result.commands.commands.is_empty());
}
