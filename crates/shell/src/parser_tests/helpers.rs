// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared test helpers for parser tests.

use crate::ast::*;

/// Extract a simple foreground command from an AndOrList.
/// Panics if the structure doesn't match expectations.
pub fn get_simple_command(and_or: &AndOrList) -> &SimpleCommand {
    match get_command(and_or) {
        Command::Simple(cmd) => cmd,
        Command::Job(_) => panic!("Expected simple command, not job"),
        Command::Subshell(_) => panic!("Expected simple command, not subshell"),
        Command::BraceGroup(_) => panic!("Expected simple command, not brace group"),
    }
}

/// Extract the command from an AndOrList (expects single foreground command).
pub fn get_command(and_or: &AndOrList) -> &Command {
    assert!(and_or.rest.is_empty(), "Expected no logical operators");
    assert!(!and_or.first.background, "Expected foreground command");
    &and_or.first.command
}

/// Extract a job from an AndOrList.
pub fn get_job(and_or: &AndOrList) -> &Job {
    match get_command(and_or) {
        Command::Job(p) => p,
        _ => panic!("Expected job"),
    }
}

/// Extract command name as string from a SimpleCommand.
pub fn cmd_name(cmd: &SimpleCommand) -> &str {
    match &cmd.name.parts[0] {
        WordPart::Literal { value, .. } => value,
        _ => panic!("Expected literal command name"),
    }
}

/// Extract a subshell from an AndOrList.
pub fn get_subshell(and_or: &AndOrList) -> &Subshell {
    match get_command(and_or) {
        Command::Subshell(s) => s,
        other => panic!("expected subshell, got {:?}", std::mem::discriminant(other)),
    }
}

/// Extract a brace group from an AndOrList.
pub fn get_brace_group(and_or: &AndOrList) -> &BraceGroup {
    match get_command(and_or) {
        Command::BraceGroup(bg) => bg,
        other => panic!("expected brace group, got {:?}", std::mem::discriminant(other)),
    }
}

/// Assert a word contains a single literal with the expected value.
/// Ignores the quote style.
pub fn assert_literal(word: &Word, expected: &str) {
    assert_eq!(word.parts.len(), 1);
    match &word.parts[0] {
        WordPart::Literal { value, .. } => assert_eq!(value, expected),
        _ => panic!("Expected single literal, got {:?}", word.parts[0]),
    }
}

/// Assert a word part is a literal with the expected value and quote style.
pub fn assert_quoted_literal(part: &WordPart, expected_value: &str, expected_style: QuoteStyle) {
    match part {
        WordPart::Literal { value, quoted } => {
            assert_eq!(value, expected_value);
            assert_eq!(*quoted, expected_style);
        }
        _ => panic!("expected literal, got {:?}", part),
    }
}
