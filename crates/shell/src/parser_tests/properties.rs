// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Property-based tests for parser invariants.

use crate::ast::*;
use crate::parser::Parser;
use crate::token::Span;
use proptest::prelude::*;

/// Strategy for generating valid shell words (alphanumeric + underscore).
fn word_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z_][a-zA-Z0-9_]{0,10}".prop_map(String::from)
}

/// Strategy for generating simple commands.
fn simple_command_strategy() -> impl Strategy<Value = String> {
    (
        word_strategy(),
        prop::collection::vec(word_strategy(), 0..5),
    )
        .prop_map(|(name, args)| {
            if args.is_empty() {
                name
            } else {
                format!("{} {}", name, args.join(" "))
            }
        })
}

/// Strategy for generating command lists.
fn command_list_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(simple_command_strategy(), 1..5).prop_map(|cmds| cmds.join("; "))
}

proptest! {
    /// Invariant: Parsing a valid command list succeeds.
    #[test]
    fn parse_valid_command_list(input in command_list_strategy()) {
        let result = Parser::parse(&input);
        prop_assert!(result.is_ok(), "Failed to parse: {:?}", input);
    }

    /// Invariant: CommandList span covers the entire input (for non-empty input).
    #[test]
    fn span_covers_input(input in command_list_strategy()) {
        if let Ok(result) = Parser::parse(&input) {
            if !result.commands.is_empty() {
                // Span should start at 0 and end at or near input length
                prop_assert!(result.span.start == 0);
                prop_assert!(result.span.end <= input.len());
            }
        }
    }

    /// Invariant: Empty input produces empty command list.
    #[test]
    fn empty_input_produces_empty_list(ws in "[ \t\n]*") {
        let result = Parser::parse(&ws).unwrap();
        prop_assert!(result.commands.is_empty());
    }

    /// Invariant: Single command produces single AndOrList.
    #[test]
    fn single_command_produces_one_and_or(cmd in simple_command_strategy()) {
        let result = Parser::parse(&cmd).unwrap();
        prop_assert_eq!(result.commands.len(), 1);
    }

    /// Invariant: N semicolon-separated commands produce N AndOrLists.
    #[test]
    fn semicolon_count_matches_command_count(
        cmds in prop::collection::vec(word_strategy(), 1..10)
    ) {
        let input = cmds.join("; ");
        let result = Parser::parse(&input).unwrap();
        prop_assert_eq!(result.commands.len(), cmds.len());
    }
}

// =============================================================================
// Span Invariant Tests
// =============================================================================

/// Verify that child spans are contained within parent spans.
fn verify_span_containment(parent: Span, child: Span, context: &str) {
    assert!(
        parent.start <= child.start && child.end <= parent.end,
        "Child span {:?} not contained in parent {:?} for {}",
        child,
        parent,
        context
    );
}

/// Recursively verify span containment in AST.
fn verify_ast_spans(cmd_list: &CommandList) {
    for and_or in &cmd_list.commands {
        verify_span_containment(cmd_list.span, and_or.span, "AndOrList in CommandList");
        verify_and_or_spans(and_or);
    }
}

fn verify_and_or_spans(and_or: &AndOrList) {
    verify_span_containment(and_or.span, and_or.first.span, "first in AndOrList");
    verify_command_item_spans(&and_or.first, and_or.span);
    for (_, item) in &and_or.rest {
        verify_span_containment(and_or.span, item.span, "rest item in AndOrList");
        verify_command_item_spans(item, and_or.span);
    }
}

fn verify_command_item_spans(item: &CommandItem, parent: Span) {
    verify_span_containment(parent, item.span, "CommandItem");
    match &item.command {
        Command::Simple(cmd) => {
            verify_span_containment(item.span, cmd.span, "SimpleCommand");
            verify_span_containment(cmd.span, cmd.name.span, "name in SimpleCommand");
            for arg in &cmd.args {
                verify_span_containment(cmd.span, arg.span, "arg in SimpleCommand");
            }
        }
        Command::Job(job) => {
            verify_span_containment(item.span, job.span, "Job");
            for cmd in &job.commands {
                verify_span_containment(job.span, cmd.span, "cmd in Job");
            }
        }
        Command::Subshell(subshell) => {
            verify_span_containment(item.span, subshell.span, "Subshell");
            verify_ast_spans(&subshell.body);
        }
        Command::BraceGroup(group) => {
            verify_span_containment(item.span, group.span, "BraceGroup");
            verify_ast_spans(&group.body);
        }
    }
}

proptest! {
    /// Invariant: All child spans are contained within parent spans.
    #[test]
    fn child_spans_contained_in_parent(input in command_list_strategy()) {
        if let Ok(result) = Parser::parse(&input) {
            verify_ast_spans(&result);
        }
    }
}

// =============================================================================
// Nesting Depth Tests
// =============================================================================

#[test]
fn test_deep_subshell_nesting() {
    // Generate 20 levels of subshell nesting
    let depth = 20;
    let open = "(".repeat(depth);
    let close = ")".repeat(depth);
    let input = format!("{}cmd{}", open, close);

    let result = Parser::parse(&input);
    assert!(result.is_ok(), "Failed to parse {}-level nesting", depth);
}

#[test]
fn test_deep_brace_group_nesting() {
    // Generate 20 levels of brace group nesting
    let depth = 20;
    let mut input = String::new();
    for _ in 0..depth {
        input.push_str("{ ");
    }
    input.push_str("cmd");
    for _ in 0..depth {
        input.push_str("; }");
    }

    let result = Parser::parse(&input);
    assert!(
        result.is_ok(),
        "Failed to parse {}-level brace nesting",
        depth
    );
}

#[test]
fn test_alternating_deep_nesting() {
    // Alternate between subshells and brace groups
    let depth = 10;
    let mut input = String::new();
    for i in 0..depth {
        if i % 2 == 0 {
            input.push('(');
        } else {
            input.push_str("{ ");
        }
    }
    input.push_str("cmd");
    for i in (0..depth).rev() {
        if i % 2 == 0 {
            input.push(')');
        } else {
            input.push_str("; }");
        }
    }

    let result = Parser::parse(&input);
    assert!(result.is_ok(), "Failed to parse alternating nesting");
}

// =============================================================================
// Job Property Tests
// =============================================================================

proptest! {
    /// Invariant: N piped commands produce a job with N commands.
    #[test]
    fn pipe_count_matches_command_count(
        cmds in prop::collection::vec(word_strategy(), 2..6)
    ) {
        let input = cmds.join(" | ");
        let result = Parser::parse(&input).unwrap();
        prop_assert_eq!(result.commands.len(), 1);

        let and_or = &result.commands[0];
        match &and_or.first.command {
            Command::Job(p) => {
                prop_assert_eq!(p.commands.len(), cmds.len());
            }
            _ => prop_assert!(false, "Expected job for piped commands"),
        }
    }

    /// Invariant: Logical operators don't affect job internal structure.
    #[test]
    fn logical_ops_preserve_job_structure(
        a in word_strategy(),
        b in word_strategy(),
        c in word_strategy(),
    ) {
        // a | b && c -> job(a, b) && c
        let input = format!("{} | {} && {}", a, b, c);
        let result = Parser::parse(&input).unwrap();
        prop_assert_eq!(result.commands.len(), 1);

        let and_or = &result.commands[0];
        prop_assert_eq!(and_or.rest.len(), 1);

        match &and_or.first.command {
            Command::Job(p) => {
                prop_assert_eq!(p.commands.len(), 2);
            }
            _ => prop_assert!(false, "Expected job"),
        }
    }
}

// =============================================================================
// Mixed Separator Property Tests
// =============================================================================

proptest! {
    /// Invariant: Different separator styles produce same command structure.
    #[test]
    fn mixed_separators_consistent(
        cmds in prop::collection::vec(word_strategy(), 2..5),
        sep_idx in 0usize..5,
    ) {
        let separators = [" ; ", "\n", " ; ; ", "\n\n", " ;\n"];
        let sep = separators[sep_idx % separators.len()];
        let input = cmds.join(sep);

        if let Ok(result) = Parser::parse(&input) {
            // Each separator style should produce the same number of commands
            prop_assert_eq!(result.commands.len(), cmds.len());
        }
    }

    /// Invariant: Leading separators are ignored.
    #[test]
    fn leading_separators_ignored(
        cmd in word_strategy(),
        leading_semis in 0usize..5,
    ) {
        let prefix = ";".repeat(leading_semis);
        let input = format!("{}{}", prefix, cmd);
        let result = Parser::parse(&input).unwrap();
        prop_assert_eq!(result.commands.len(), 1);
    }

    /// Invariant: Trailing separators are ignored.
    #[test]
    fn trailing_separators_ignored(
        cmd in word_strategy(),
        trailing_semis in 0usize..5,
    ) {
        let suffix = ";".repeat(trailing_semis);
        let input = format!("{}{}", cmd, suffix);
        let result = Parser::parse(&input).unwrap();
        prop_assert_eq!(result.commands.len(), 1);
    }
}

// =============================================================================
// Subshell Property Tests
// =============================================================================

proptest! {
    /// Invariant: N-level nested subshells produce N-level AST nesting.
    #[test]
    fn subshell_nesting_depth_correct(depth in 1usize..10) {
        let open = "(".repeat(depth);
        let close = ")".repeat(depth);
        let input = format!("{}cmd{}", open, close);

        let result = Parser::parse(&input).unwrap();
        prop_assert_eq!(result.commands.len(), 1);

        // Verify depth by counting subshell levels
        let mut current = &result.commands[0].first.command;
        for level in 0..depth {
            match current {
                Command::Subshell(s) => {
                    if level == depth - 1 {
                        // Innermost should contain simple command
                        prop_assert_eq!(s.body.commands.len(), 1);
                    } else {
                        // Intermediate should contain another subshell
                        prop_assert_eq!(s.body.commands.len(), 1);
                        current = &s.body.commands[0].first.command;
                    }
                }
                Command::Simple(_) if level == depth - 1 => {
                    // This is fine - innermost is a simple command
                    // (happens when depth=1 sometimes)
                }
                _ => prop_assert!(false, "Unexpected command type at level {}", level),
            }
        }
    }

    /// Invariant: Commands inside subshell count matches.
    #[test]
    fn subshell_command_count(
        cmds in prop::collection::vec(word_strategy(), 1..5)
    ) {
        let inner = cmds.join("; ");
        let input = format!("({})", inner);

        let result = Parser::parse(&input).unwrap();
        prop_assert_eq!(result.commands.len(), 1);

        match &result.commands[0].first.command {
            Command::Subshell(s) => {
                prop_assert_eq!(s.body.commands.len(), cmds.len());
            }
            _ => prop_assert!(false, "Expected subshell"),
        }
    }
}

// =============================================================================
// Error Recovery Property Tests
// =============================================================================

proptest! {
    /// Invariant: Recovery mode finds valid commands even with errors.
    #[test]
    fn recovery_preserves_valid_commands(
        valid_cmds in prop::collection::vec(word_strategy(), 1..4)
    ) {
        // Create input with error tokens between valid commands
        // e.g., "cmd1 ; | ; cmd2 ; && ; cmd3"
        let mut parts = vec![];
        for (i, cmd) in valid_cmds.iter().enumerate() {
            parts.push(cmd.clone());
            if i < valid_cmds.len() - 1 {
                // Insert error token between valid commands
                parts.push("|".to_string());
            }
        }
        let input = parts.join(" ; ");

        let result = Parser::parse_with_recovery(&input);

        // Should find all valid commands
        prop_assert_eq!(
            result.commands.commands.len(),
            valid_cmds.len(),
            "Expected {} valid commands in recovery mode",
            valid_cmds.len()
        );

        // Should have errors for the | tokens
        prop_assert!(
            result.errors.len() >= valid_cmds.len() - 1,
            "Should have at least {} errors",
            valid_cmds.len() - 1
        );
    }
}

// =============================================================================
// Variable Expansion Property Tests
// =============================================================================

proptest! {
    /// Invariant: Variable names in words produce Variable WordParts.
    #[test]
    fn variable_expansion_produces_variable_part(
        name in "[A-Z_][A-Z0-9_]{0,10}"
    ) {
        let input = format!("echo ${}", name);
        let result = Parser::parse(&input).unwrap();

        prop_assert_eq!(result.commands.len(), 1);
        let cmd = match &result.commands[0].first.command {
            Command::Simple(c) => c,
            _ => panic!("Expected simple command"),
        };

        // The second word (first arg) should contain a variable
        prop_assert_eq!(cmd.args.len(), 1);
        let has_variable = cmd.args[0].parts.iter().any(|part| {
            matches!(part, WordPart::Variable { .. })
        });
        prop_assert!(has_variable, "Should have variable part for ${}", name);
    }
}

// =============================================================================
// Robustness Tests - Parser/Lexer Never Panics
// =============================================================================

use crate::lexer::Lexer;
use crate::token::context_snippet;

proptest! {
    /// Invariant: Lexer never panics on arbitrary ASCII input.
    #[test]
    fn lexer_never_panics(input in "[ -~\\n\\t]{0,200}") {
        // Just check it doesn't panic - result can be Ok or Err
        let _ = Lexer::tokenize(&input);
    }

    /// Invariant: Parser never panics on arbitrary ASCII input.
    #[test]
    fn parser_never_panics(input in "[ -~\\n\\t]{0,200}") {
        let _ = Parser::parse(&input);
        let _ = Parser::parse_with_recovery(&input);
    }

    /// Invariant: Lexer errors have valid spans within input bounds.
    #[test]
    fn lexer_errors_have_valid_spans(input in "[ -~]{0,100}") {
        if let Err(e) = Lexer::tokenize(&input) {
            let span = e.span();
            prop_assert!(span.start <= input.len(), "span.start out of bounds");
            prop_assert!(span.end <= input.len(), "span.end out of bounds");
            prop_assert!(span.start <= span.end, "span.start > span.end");
        }
    }

    /// Invariant: Parser errors have valid spans (when present) within input bounds.
    #[test]
    fn parser_errors_have_valid_spans(input in "[ -~]{0,100}") {
        if let Err(e) = Parser::parse(&input) {
            if let Some(span) = e.span() {
                prop_assert!(span.start <= input.len(), "span.start out of bounds");
                prop_assert!(span.end <= input.len(), "span.end out of bounds");
                prop_assert!(span.start <= span.end, "span.start > span.end");
            }
        }
    }

    /// Invariant: context_snippet never panics for any valid span.
    #[test]
    fn context_snippet_never_panics(
        input in "[ -~\\n\\t]{0,200}",
        start in 0usize..100,
        len in 0usize..50
    ) {
        // Clamp values to input bounds
        let clamped_start = start.min(input.len());
        let clamped_end = (clamped_start + len).min(input.len());
        let span = Span::new(clamped_start, clamped_end);
        // Should not panic
        let _ = context_snippet(&input, span, 20);
    }

    /// Invariant: Error diagnostics never panic.
    #[test]
    fn error_diagnostic_never_panics(input in "[ -~\\n\\t]{0,100}") {
        if let Err(e) = Lexer::tokenize(&input) {
            let _ = e.diagnostic(&input);
            let _ = e.context(&input, 20);
        }
        if let Err(e) = Parser::parse(&input) {
            let _ = e.diagnostic(&input);
            let _ = e.context(&input, 20);
        }
    }
}

// =============================================================================
// Unicode Robustness Tests
// =============================================================================

proptest! {
    /// Invariant: Lexer handles mixed ASCII and Unicode without panicking.
    #[test]
    fn lexer_handles_unicode(input in "[a-z日本語\\s]{0,50}") {
        let _ = Lexer::tokenize(&input);
    }

    /// Invariant: Parser handles mixed ASCII and Unicode without panicking.
    #[test]
    fn parser_handles_unicode(input in "[a-z日本語\\s]{0,50}") {
        let _ = Parser::parse(&input);
    }
}

// =============================================================================
// Recovery Property Tests
// =============================================================================

proptest! {
    /// Invariant: Recovery mode never panics and collects errors properly.
    #[test]
    fn recovery_never_panics_and_collects(input in "[ -~\\n\\t]{0,150}") {
        let result = Parser::parse_with_recovery(&input);
        // Result should be valid struct
        let _ = result.commands;
        let _ = result.errors;
    }

    /// Invariant: Recovery errors have valid spans.
    #[test]
    fn recovery_errors_have_valid_spans(input in "[ -~]{0,100}") {
        let result = Parser::parse_with_recovery(&input);
        for err in &result.errors {
            if let Some(span) = err.span() {
                prop_assert!(span.start <= input.len());
                prop_assert!(span.end <= input.len());
            }
        }
    }
}
