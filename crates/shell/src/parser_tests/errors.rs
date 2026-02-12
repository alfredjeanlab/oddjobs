// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::helpers::get_simple_command;
use super::macros::parse_error_tests;
use crate::ast::{Command, WordPart};
use crate::parser::{ParseError, Parser};
use crate::token::TokenKind;

parse_error_tests! {
    macro_pipe_at_start: "| cmd" => ParseError::UnexpectedToken { .. },
    macro_and_at_end: "cmd &&" => ParseError::UnexpectedEof { .. },
    macro_or_at_end: "cmd ||" => ParseError::UnexpectedEof { .. },
}

#[test]
fn test_error_span() {
    let err = Parser::parse("| cmd").unwrap_err();
    let span = err.span();
    assert!(span.is_some());
    let span = span.unwrap();
    assert_eq!(span.start, 0);
    assert_eq!(span.end, 1);
}

#[test]
fn test_error_span_eof() {
    let err = Parser::parse("cmd &&").unwrap_err();
    // UnexpectedEof has no span
    assert!(err.span().is_none());
}

#[test]
fn test_error_context() {
    let input = "echo | | bad";
    let result = Parser::parse(input);

    if let Err(e) = result {
        let context = e.context(input, 20).unwrap();
        assert!(context.contains("echo | | bad"));
        assert!(context.contains("^"));
    }
}

#[test]
fn test_error_context_at_start() {
    let input = "| cmd";
    let err = Parser::parse(input).unwrap_err();
    let context = err.context(input, 10).unwrap();
    // Should show the pipe at position 0
    assert!(context.starts_with("| cmd"));
    assert!(context.contains("^"));
}

#[test]
fn test_error_context_at_end() {
    let input = "cmd &&";
    let err = Parser::parse(input).unwrap_err();
    // UnexpectedEof has no span, so no context
    assert!(err.context(input, 10).is_none());
}

#[test]
fn test_error_context_utf8() {
    let input = "echo \u{65e5}\u{672c}\u{8a9e} | | bad";
    let result = Parser::parse(input);

    if let Err(e) = result {
        let context = e.context(input, 30);
        assert!(context.is_some());
    }
}

#[test]
fn test_unexpected_pipe_at_start() {
    let err = Parser::parse("| cmd").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { found: TokenKind::Pipe, .. }));
}

#[test]
fn test_unexpected_and_at_start() {
    let err = Parser::parse("&& cmd").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { found: TokenKind::And, .. }));
}

#[test]
fn test_unexpected_or_at_start() {
    let err = Parser::parse("|| cmd").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { found: TokenKind::Or, .. }));
}

#[test]
fn test_unexpected_ampersand_at_start() {
    let err = Parser::parse("& cmd").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { found: TokenKind::Ampersand, .. }));
}

#[test]
fn test_recovery_after_error() {
    let result = Parser::parse_with_recovery("| ; echo ok");
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.commands.commands.len(), 1);

    let cmd = get_simple_command(&result.commands.commands[0]);
    assert_eq!(cmd.name.parts, vec![WordPart::literal("echo")]);
}

#[test]
fn test_recovery_multiple_errors() {
    let result = Parser::parse_with_recovery("| ; && ; echo ok");
    assert_eq!(result.errors.len(), 2);
    assert_eq!(result.commands.commands.len(), 1);
}

#[test]
fn test_recovery_valid_between_errors() {
    let result = Parser::parse_with_recovery("| ; echo middle ; && ; echo end");
    // Errors: |, &&
    assert_eq!(result.errors.len(), 2);
    // Valid commands: echo middle, echo end
    assert_eq!(result.commands.commands.len(), 2);
}

#[test]
fn test_lexer_error_propagation() {
    // Empty variable should cause lexer error
    let result = Parser::parse_with_recovery("echo $");
    assert_eq!(result.errors.len(), 1);
    assert!(matches!(result.errors[0], ParseError::Lexer(_)));
}

#[test]
fn test_pipe_now_supported() {
    // Pipes are now supported, so this should parse successfully
    let result = Parser::parse("cmd1 | cmd2").unwrap();
    assert_eq!(result.commands.len(), 1);
    // The result is a job
    let and_or = &result.commands[0];
    assert!(and_or.rest.is_empty());
    match &and_or.first.command {
        Command::Job(p) => assert_eq!(p.commands.len(), 2),
        _ => panic!("Expected job"),
    }
}

#[test]
fn test_logical_or_now_supported() {
    // Logical OR is now supported, so this should parse successfully
    let result = Parser::parse("cmd1 || cmd2").unwrap();
    assert_eq!(result.commands.len(), 1);
    let and_or = &result.commands[0];
    assert_eq!(and_or.rest.len(), 1);
}

#[test]
fn test_pipe_at_end_error() {
    // Pipe at end should be an error
    let err = Parser::parse("cmd |").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedEof { .. }));
}

#[test]
fn test_and_at_end_error() {
    // AND at end should be an error
    let err = Parser::parse("cmd &&").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedEof { .. }));
}

#[test]
fn test_or_at_end_error() {
    // OR at end should be an error
    let err = Parser::parse("cmd ||").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedEof { .. }));
}

#[test]
fn test_recovery_preserves_valid_commands() {
    let result = Parser::parse_with_recovery("valid1 ; | ; valid2 ; && ; valid3");

    // Should have 3 valid commands
    assert_eq!(result.commands.commands.len(), 3);

    // Should have 2 errors (| and &&)
    assert_eq!(result.errors.len(), 2);

    // Verify the commands
    let commands: Vec<_> = result
        .commands
        .commands
        .iter()
        .map(|and_or| {
            let cmd = get_simple_command(and_or);
            match &cmd.name.parts[0] {
                WordPart::Literal { value, .. } => value.clone(),
                _ => panic!("Expected literal"),
            }
        })
        .collect();

    assert_eq!(commands, vec!["valid1", "valid2", "valid3"]);
}

#[test]
fn test_error_message_format() {
    let err = Parser::parse("| cmd").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("unexpected token"));
    assert!(msg.contains("'|'"));
    assert!(msg.contains("command"));
}

#[test]
fn test_recovery_with_all_errors() {
    let result = Parser::parse_with_recovery("| ; && ; ||");
    // All are errors, no valid commands
    assert!(result.commands.commands.is_empty());
    assert_eq!(result.errors.len(), 3);
}

#[yare::parameterized(
    double_pipe    = { "cmd || ||", TokenKind::Or },
    double_and     = { "cmd && &&", TokenKind::And },
    pipe_after_and = { "cmd && |",  TokenKind::Pipe },
)]
fn double_operator_error(input: &str, expected_token: TokenKind) {
    let err = Parser::parse(input).unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { found, .. } if found == expected_token));
}

#[test]
fn test_empty_subshell_allowed() {
    // This parser allows empty subshells
    let result = Parser::parse("( )");
    assert!(result.is_ok(), "Empty subshell is allowed: {:?}", result);
    let cmds = result.unwrap();
    assert_eq!(cmds.commands.len(), 1);
    match &cmds.commands[0].first.command {
        Command::Subshell(s) => {
            assert!(s.body.commands.is_empty(), "Body should be empty");
        }
        _ => panic!("Expected subshell"),
    }
}

#[test]
fn test_empty_brace_group_allowed() {
    // This parser allows empty brace groups
    let result = Parser::parse("{ }");
    assert!(result.is_ok(), "Empty brace group is allowed: {:?}", result);
    let cmds = result.unwrap();
    assert_eq!(cmds.commands.len(), 1);
    match &cmds.commands[0].first.command {
        Command::BraceGroup(bg) => {
            assert!(bg.body.commands.is_empty(), "Body should be empty");
        }
        _ => panic!("Expected brace group"),
    }
}

#[yare::parameterized(
    unclosed_subshell    = { "(echo hello" },
    unclosed_brace_group = { "{ echo hello" },
)]
fn unclosed_group(input: &str) {
    let err = Parser::parse(input).unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedEof { .. }));
}

#[yare::parameterized(
    stray_rparen = { "echo hello )", TokenKind::RParen },
    stray_rbrace = { "echo hello }", TokenKind::RBrace },
)]
fn stray_closing_char(input: &str, expected: TokenKind) {
    let err = Parser::parse(input).unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { found, .. } if found == expected));
}

#[test]
fn test_nested_subshell_requires_separator() {
    // Nested subshells require separator or line ending
    // This is because `(inner)` inside `(echo ...)` is parsed differently
    // Valid: `(echo; (inner))` or `((inner))`
    let result = Parser::parse("((inner))");
    assert!(result.is_ok(), "Nested subshell with separator should parse: {:?}", result);
}

#[test]
fn test_subshell_not_in_job_segment() {
    // Subshell followed by pipe is not supported in this parser
    // (subshells are compound commands, not simple commands)
    let err = Parser::parse("(echo hello) | cat").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { found: TokenKind::Pipe, .. }));
}

#[test]
fn test_deeply_nested_unclosed() {
    // Multiple levels of unclosed parens
    let err = Parser::parse("((( echo").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedEof { .. }));
}

//
// The parser does NOT have special handling for shell control flow keywords.
// Keywords like 'if', 'for', 'while', 'case' are treated as regular command
// names. These tests document the current behavior.

#[yare::parameterized(
    keyword_if    = { "if" },
    keyword_for   = { "for" },
    keyword_while = { "while" },
    keyword_case  = { "case" },
    keyword_then  = { "then" },
    keyword_fi    = { "fi" },
)]
fn keyword_treated_as_command(keyword: &str) {
    let result = Parser::parse(keyword).unwrap();
    assert_eq!(result.commands.len(), 1);
    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.name.parts, vec![WordPart::literal(keyword)]);
}

#[test]
fn test_do_done_treated_as_commands() {
    // "do" and "done" are parsed as separate commands
    let result = Parser::parse("do ; done").unwrap();
    assert_eq!(result.commands.len(), 2);

    let cmd1 = get_simple_command(&result.commands[0]);
    let cmd2 = get_simple_command(&result.commands[1]);
    assert_eq!(cmd1.name.parts, vec![WordPart::literal("do")]);
    assert_eq!(cmd2.name.parts, vec![WordPart::literal("done")]);
}

#[test]
fn test_if_statement_parsed_as_sequence() {
    // "if true; then echo; fi" parses as sequence of commands
    // This documents that full if/then/fi syntax is NOT specially handled
    let result = Parser::parse("if true; then echo; fi").unwrap();
    // The structure depends on how semicolons separate things
    // if, true, then, echo, fi are all separate commands
    assert!(result.commands.len() >= 3, "should have multiple commands");
}

#[test]
fn test_for_loop_parsed_as_sequence() {
    // "for i in a b; do echo $i; done" parses as commands
    let result = Parser::parse("for i in a b; do echo $i; done").unwrap();
    assert!(result.commands.len() >= 2, "should have multiple commands");
}

//
// Function definitions (foo() { ... }) are NOT supported.
// The parser expects ';' or newline after a command, not '('.

#[test]
fn test_function_like_syntax_error() {
    // "foo()" causes an error: after "foo", parser expects separator not '('
    let err = Parser::parse("foo()").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { found: TokenKind::LParen, .. }));
}

#[test]
fn test_function_with_body_error() {
    // "foo() { echo; }" also errors on the '('
    let err = Parser::parse("foo() { echo; }").unwrap_err();
    assert!(matches!(err, ParseError::UnexpectedToken { found: TokenKind::LParen, .. }));
}

//
// [[ expr ]] double-bracket conditionals are NOT specially handled.
// [ expr ] single-bracket is the test command.

#[test]
fn test_single_bracket_test() {
    // [ is a command (alias for 'test'), ] is an argument
    let result = Parser::parse("[ -f file ]").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.name.parts, vec![WordPart::literal("[")]);
    // -f, file, ] are arguments
    assert_eq!(cmd.args.len(), 3);
}

#[test]
fn test_double_bracket_not_special() {
    // [[ is not specially handled - parsed as a word
    let result = Parser::parse("[[ -f file ]]").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.name.parts, vec![WordPart::literal("[[")]);
}

//
// The lexer correctly tokenizes heredocs (<<EOF...EOF), but the parser does
// NOT yet handle HereDoc tokens. The parser expects simple command structure.
// This documents the current limitation.

#[test]
fn test_heredoc_now_supported() {
    // Heredocs are now supported by the parser
    let input = "cat <<EOF\nline\nEOF";
    let result = Parser::parse(input);

    // Heredocs should parse successfully
    assert!(result.is_ok(), "heredocs are now supported by parser");
    let ast = result.unwrap();
    assert_eq!(ast.commands.len(), 1);

    // Verify it's a simple command with a heredoc redirection
    let cmd = match &ast.commands[0].first.command {
        crate::ast::Command::Simple(c) => c,
        _ => panic!("Expected simple command"),
    };
    assert_eq!(cmd.redirections.len(), 1);
    assert!(
        matches!(&cmd.redirections[0], crate::ast::Redirection::HereDoc { .. }),
        "expected HereDoc redirection"
    );
}

#[test]
fn test_multiline_with_continuation() {
    // Line continuation is handled by lexer, parser should see connected input
    let result = Parser::parse("echo \\\nhello").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.name.parts, vec![WordPart::literal("echo")]);
    assert_eq!(cmd.args.len(), 1);
    assert_eq!(cmd.args[0].parts, vec![WordPart::literal("hello")]);
}

#[test]
fn test_error_multiline_span_locates_correct_line() {
    let input = "echo hello\necho world\n| bad";
    let err = Parser::parse(input).unwrap_err();
    let diag = err.diagnostic(input);
    assert!(diag.is_some(), "Should have diagnostic for error with span");
    let diag = diag.unwrap();
    assert!(diag.contains("line 3"), "Error is on line 3: {}", diag);
}

#[test]
fn test_error_multiline_recovery() {
    let input = "cmd1\n| bad\ncmd2";
    let result = Parser::parse_with_recovery(input);
    // Should recover: cmd1 parses, | is error, cmd2 parses
    assert_eq!(result.errors.len(), 1, "Should have 1 error");
    assert_eq!(result.commands.commands.len(), 2, "Should have 2 valid commands");
}

#[test]
fn test_substitution_error_preserves_context() {
    let err = Parser::parse("echo $(| bad)").unwrap_err();
    // Error should be wrapped in InSubstitution
    if let ParseError::InSubstitution { inner, span } = err {
        assert!(
            matches!(*inner, ParseError::UnexpectedToken { .. }),
            "Inner should be UnexpectedToken"
        );
        assert!(span.start > 0, "Span should point to substitution start");
    } else {
        panic!("Expected InSubstitution error, got: {:?}", err);
    }
}

#[test]
fn test_nested_substitution_error() {
    let err = Parser::parse("echo $(echo $(| inner))").unwrap_err();
    // Should get InSubstitution wrapping an InSubstitution
    if let ParseError::InSubstitution { inner, .. } = err {
        if let ParseError::InSubstitution { inner: inner2, .. } = *inner {
            assert!(
                matches!(*inner2, ParseError::UnexpectedToken { .. }),
                "Innermost should be UnexpectedToken"
            );
        } else {
            panic!("Expected nested InSubstitution, got: {:?}", inner);
        }
    } else {
        panic!("Expected InSubstitution error, got: {:?}", err);
    }
}

#[test]
fn test_recovery_preserves_error_order() {
    let result = Parser::parse_with_recovery("| ; && ; ||");
    assert_eq!(result.errors.len(), 3);

    // Errors should be in order of occurrence
    let span0 = result.errors[0].span().unwrap();
    let span1 = result.errors[1].span().unwrap();
    let span2 = result.errors[2].span().unwrap();

    assert!(span0.start < span1.start, "Error 0 should come before error 1");
    assert!(span1.start < span2.start, "Error 1 should come before error 2");
}

#[test]
fn test_lexer_error_stops_parsing() {
    // Lexer error should stop parsing completely
    let result = Parser::parse_with_recovery("echo $ ; valid");
    assert_eq!(result.errors.len(), 1);
    assert!(matches!(result.errors[0], ParseError::Lexer(_)));
    // Parser stops at lexer error, so no valid commands parsed
    assert!(result.commands.commands.is_empty());
}

#[test]
fn test_diagnostic_method() {
    let input = "| cmd";
    let err = Parser::parse(input).unwrap_err();
    let diag = err.diagnostic(input).unwrap();
    assert!(diag.contains("error:"));
    assert!(diag.contains("line 1"));
    assert!(diag.contains("| cmd"));
    assert!(diag.contains("^"));
}

#[test]
fn test_diagnostic_eof_returns_none() {
    let input = "cmd &&";
    let err = Parser::parse(input).unwrap_err();
    // UnexpectedEof has no span, so diagnostic returns None
    assert!(err.diagnostic(input).is_none());
}

#[test]
fn test_diagnostic_multiline_shows_correct_line() {
    let input = "line1\nline2\n&& bad";
    let err = Parser::parse(input).unwrap_err();
    let diag = err.diagnostic(input).unwrap();
    assert!(diag.contains("line 3"), "Should show line 3: {}", diag);
    assert!(diag.contains("&& bad"), "Should show line content: {}", diag);
}
