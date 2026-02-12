// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Variable lexer tests: simple variables, braced variables, modifiers, error cases.

use crate::lexer::{Lexer, LexerError};
use crate::token::{Span, TokenKind};

lex_tests! {
    simple_variable: "$HOME" => [
        TokenKind::Variable { name: "HOME".into(), modifier: None },
    ],
    variable_with_underscore: "$MY_VAR" => [
        TokenKind::Variable { name: "MY_VAR".into(), modifier: None },
    ],
    variable_starting_with_underscore: "$_private" => [
        TokenKind::Variable { name: "_private".into(), modifier: None },
    ],
    variable_with_numbers: "$VAR123" => [
        TokenKind::Variable { name: "VAR123".into(), modifier: None },
    ],
    var_name_underscore: "$VAR_NAME" => [
        TokenKind::Variable { name: "VAR_NAME".into(), modifier: None },
    ],
    var_hyphen_terminates: "$VAR-NAME" => [
        TokenKind::Variable { name: "VAR".into(), modifier: None },
        TokenKind::Word("-NAME".into()),
    ],
    consecutive_variables: "$A$B$C" => [
        TokenKind::Variable { name: "A".into(), modifier: None },
        TokenKind::Variable { name: "B".into(), modifier: None },
        TokenKind::Variable { name: "C".into(), modifier: None },
    ],
    variable_followed_by_and: "$VAR&&" => [
        TokenKind::Variable { name: "VAR".into(), modifier: None },
        TokenKind::And,
    ],
}

span_tests! {
    simple_variable_span: "$HOME" => [(0, 5)],
}

#[test]
fn test_variable_terminates_at_special_chars() {
    for (input, var_name, word) in [
        ("$VAR.txt", "VAR", ".txt"),
        ("$VAR/path", "VAR", "/path"),
        ("$VAR:value", "VAR", ":value"),
        ("$VAR=value", "VAR", "=value"),
    ] {
        let tokens = Lexer::tokenize(input).unwrap();
        assert_eq!(tokens.len(), 2, "input: {}", input);
        assert_eq!(tokens[0].kind, TokenKind::Variable { name: var_name.into(), modifier: None });
        assert_eq!(tokens[1].kind, TokenKind::Word(word.into()));
    }
}

lex_tests! {
    braced_variable: "${HOME}" => [
        TokenKind::Variable { name: "HOME".into(), modifier: None },
    ],
    braced_variable_adjacent_to_text: "${HOME}/bin" => [
        TokenKind::Variable { name: "HOME".into(), modifier: None },
        TokenKind::Word("/bin".into()),
    ],
    braced_variable_adjacent_text_both_sides: "x${VAR}y" => [
        TokenKind::Word("x".into()),
        TokenKind::Variable { name: "VAR".into(), modifier: None },
        TokenKind::Word("y".into()),
    ],
    alternating_braced_vars_and_text: "${A}B${C}D" => [
        TokenKind::Variable { name: "A".into(), modifier: None },
        TokenKind::Word("B".into()),
        TokenKind::Variable { name: "C".into(), modifier: None },
        TokenKind::Word("D".into()),
    ],
    two_consecutive_braced_vars: "${VAR}${OTHER}" => [
        TokenKind::Variable { name: "VAR".into(), modifier: None },
        TokenKind::Variable { name: "OTHER".into(), modifier: None },
    ],
}

span_tests! {
    braced_variable_span: "${HOME}" => [(0, 7)],
}

lex_tests! {
    variable_default_value: "${HOME:-/tmp}" => [
        TokenKind::Variable { name: "HOME".into(), modifier: Some(":-/tmp".into()) },
    ],
    variable_assign_default: "${VAR:=default}" => [
        TokenKind::Variable { name: "VAR".into(), modifier: Some(":=default".into()) },
    ],
    variable_nested_in_modifier: "${VAR:-${OTHER}}" => [
        TokenKind::Variable { name: "VAR".into(), modifier: Some(":-${OTHER}".into()) },
    ],
    variable_use_alternative: "${VAR:+value}" => [
        TokenKind::Variable { name: "VAR".into(), modifier: Some(":+value".into()) },
    ],
    deeply_nested_defaults: "${A:-${B:-${C}}}" => [
        TokenKind::Variable { name: "A".into(), modifier: Some(":-${B:-${C}}".into()) },
    ],
    variable_error_if_unset: "${VAR:?error message}" => [
        TokenKind::Variable { name: "VAR".into(), modifier: Some(":?error message".into()) },
    ],
    multiple_nested_vars_in_modifier: "${VAR:-${X}${Y}}" => [
        TokenKind::Variable { name: "VAR".into(), modifier: Some(":-${X}${Y}".into()) },
    ],
    variable_prefix_removal: "${VAR#pattern}" => [
        TokenKind::Variable { name: "VAR".into(), modifier: Some("#pattern".into()) },
    ],
    literal_braces_in_modifier: "${VAR:-{literal}}" => [
        TokenKind::Variable { name: "VAR".into(), modifier: Some(":-{literal}".into()) },
    ],
    variable_suffix_removal: "${VAR%pattern}" => [
        TokenKind::Variable { name: "VAR".into(), modifier: Some("%pattern".into()) },
    ],
    modifier_followed_by_text: "${VAR:-a}rest" => [
        TokenKind::Variable { name: "VAR".into(), modifier: Some(":-a".into()) },
        TokenKind::Word("rest".into()),
    ],
}

#[test]
fn test_all_modifier_operators() {
    for (input, expected_mod) in [
        ("${VAR:-default}", ":-default"),
        ("${VAR:=default}", ":=default"),
        ("${VAR:+value}", ":+value"),
        ("${VAR:?error}", ":?error"),
        ("${VAR#pattern}", "#pattern"),
        ("${VAR##pattern}", "##pattern"),
        ("${VAR%suffix}", "%suffix"),
        ("${VAR%%suffix}", "%%suffix"),
    ] {
        let tokens = Lexer::tokenize(input).unwrap();
        assert_eq!(tokens.len(), 1, "input: {}", input);
        assert_eq!(
            tokens[0].kind,
            TokenKind::Variable { name: "VAR".into(), modifier: Some(expected_mod.into()) }
        );
    }
}

lex_error_tests! {
    empty_variable_dollar_only: "$" => LexerError::EmptyVariable { .. },
    empty_variable_dollar_space: "$ " => LexerError::EmptyVariable { .. },
    empty_variable_operator: "$&&" => LexerError::EmptyVariable { .. },
    unterminated_brace_eof: "${VAR" => LexerError::UnterminatedVariable { .. },
    unterminated_modifier: "${VAR:-default" => LexerError::UnterminatedVariable { .. },
    unterminated_nested: "${VAR:-${OTHER}" => LexerError::UnterminatedVariable { .. },
    invalid_variable_starts_with_number: "${123}" => LexerError::InvalidVariableName { .. },
    invalid_name_starts_with_hyphen: "${-name}" => LexerError::InvalidVariableName { .. },
    invalid_name_unicode: "${日本}" => LexerError::InvalidVariableName { .. },
    error_dollar_in_command: "echo $" => LexerError::EmptyVariable { .. },
    unterminated_brace_after_dollar: "${" => LexerError::UnterminatedVariable { .. },
}

#[test]
fn test_empty_braced_variable() {
    let result = Lexer::tokenize("${}");
    assert!(matches!(
        result,
        Err(LexerError::EmptyVariable { span }) if span == Span::new(0, 3)
    ));
}

#[test]
fn test_empty_variable_dollar_only_span() {
    let result = Lexer::tokenize("$");
    assert!(matches!(
        result,
        Err(LexerError::EmptyVariable { span }) if span == Span::new(0, 1)
    ));
}

#[test]
fn test_error_span_accuracy_unterminated() {
    let result = Lexer::tokenize("${LONGNAME");
    if let Err(LexerError::UnterminatedVariable { span }) = result {
        assert_eq!(span.start, 0);
        assert!(span.end >= 10);
    } else {
        panic!("Expected UnterminatedVariable error");
    }
}

lex_tests! {
    variable_in_command: "echo $HOME" => [
        TokenKind::Word("echo".into()),
        TokenKind::Variable { name: "HOME".into(), modifier: None },
    ],
    variable_with_operators: "$A && $B" => [
        TokenKind::Variable { name: "A".into(), modifier: None },
        TokenKind::And,
        TokenKind::Variable { name: "B".into(), modifier: None },
    ],
    variable_with_pipe: "echo $VAR | cat" => [
        TokenKind::Word("echo".into()),
        TokenKind::Variable { name: "VAR".into(), modifier: None },
        TokenKind::Pipe,
        TokenKind::Word("cat".into()),
    ],
    variable_with_semicolon: "$A; $B" => [
        TokenKind::Variable { name: "A".into(), modifier: None },
        TokenKind::Semi,
        TokenKind::Variable { name: "B".into(), modifier: None },
    ],
    variable_with_newline: "$A\n$B" => [
        TokenKind::Variable { name: "A".into(), modifier: None },
        TokenKind::Newline,
        TokenKind::Variable { name: "B".into(), modifier: None },
    ],
    variable_after_pipe: "cmd | $PAGER" => [
        TokenKind::Word("cmd".into()),
        TokenKind::Pipe,
        TokenKind::Variable { name: "PAGER".into(), modifier: None },
    ],
}

lex_tests! {
    simple_variable_followed_by_slash: "$HOME/bin" => [
        TokenKind::Variable { name: "HOME".into(), modifier: None },
        TokenKind::Word("/bin".into()),
    ],
    variable_followed_by_dot: "$FILE.txt" => [
        TokenKind::Variable { name: "FILE".into(), modifier: None },
        TokenKind::Word(".txt".into()),
    ],
    braced_variable_in_path: "/usr/${LOCAL}/bin" => [
        TokenKind::Word("/usr/".into()),
        TokenKind::Variable { name: "LOCAL".into(), modifier: None },
        TokenKind::Word("/bin".into()),
    ],
    variable_with_deeply_nested_braces: "${VAR:-${A:-${B}}}" => [
        TokenKind::Variable { name: "VAR".into(), modifier: Some(":-${A:-${B}}".into()) },
    ],
}

#[test]
fn test_variable_nesting_depth_20() {
    let mut input = String::new();
    for i in 0..20 {
        input.push_str(&format!("${{V{}:-", i));
    }
    input.push_str("default");
    for _ in 0..20 {
        input.push('}');
    }
    let tokens = Lexer::tokenize(&input).unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(matches!(tokens[0].kind, TokenKind::Variable { .. }));
}

#[test]
fn test_variable_nesting_depth_50() {
    let mut input = String::new();
    for i in 0..50 {
        input.push_str(&format!("${{V{}:-", i));
    }
    input.push_str("default");
    for _ in 0..50 {
        input.push('}');
    }
    let result = Lexer::tokenize(&input);
    assert!(result.is_ok(), "Expected success, got: {:?}", result);
    let tokens = result.unwrap();
    assert_eq!(tokens.len(), 1);
}

#[test]
fn test_ast_preserves_nested_variable_verbatim() {
    let tokens = Lexer::tokenize("${A:-${B}}").unwrap();
    assert_eq!(tokens.len(), 1);
    if let TokenKind::Variable { name, modifier } = &tokens[0].kind {
        assert_eq!(name, "A");
        assert_eq!(modifier.as_ref().unwrap(), ":-${B}");
    } else {
        panic!("Expected Variable token");
    }
}

#[test]
fn test_ast_preserves_deeply_nested_variable() {
    let tokens = Lexer::tokenize("${A:-${B:-${C}}}").unwrap();
    assert_eq!(tokens.len(), 1);
    if let TokenKind::Variable { name, modifier } = &tokens[0].kind {
        assert_eq!(name, "A");
        assert_eq!(modifier.as_ref().unwrap(), ":-${B:-${C}}");
    } else {
        panic!("Expected Variable token");
    }
}

lex_error_tests! {
    error_nested_variable_unterminated: "${A:-${B:-${C}" => LexerError::UnterminatedVariable { .. },
    error_literal_braces_unbalanced: "${VAR:-{incomplete" => LexerError::UnterminatedVariable { .. },
}

#[test]
fn test_error_variable_span_points_to_outermost() {
    let result = Lexer::tokenize("${A:-${B:-${C}");
    if let Err(LexerError::UnterminatedVariable { span }) = result {
        assert_eq!(span.start, 0, "Span should point to outermost ${{");
    } else {
        panic!("Expected UnterminatedVariable error, got: {:?}", result);
    }
}

lex_tests! {
    special_variable_exit_code: "$?" => [
        TokenKind::Variable { name: "?".into(), modifier: None },
    ],
    special_variable_pid: "$$" => [
        TokenKind::Variable { name: "$".into(), modifier: None },
    ],
    special_variable_arg_count: "$#" => [
        TokenKind::Variable { name: "#".into(), modifier: None },
    ],
    special_variable_script_name: "$0" => [
        TokenKind::Variable { name: "0".into(), modifier: None },
    ],
    special_variable_braced_exit_code: "${?}" => [
        TokenKind::Variable { name: "?".into(), modifier: None },
    ],
    special_variable_braced_pid: "${$}" => [
        TokenKind::Variable { name: "$".into(), modifier: None },
    ],
    special_variable_braced_arg_count: "${#}" => [
        TokenKind::Variable { name: "#".into(), modifier: None },
    ],
    special_variable_braced_script_name: "${0}" => [
        TokenKind::Variable { name: "0".into(), modifier: None },
    ],
    special_variable_with_modifier: "${?:-default}" => [
        TokenKind::Variable { name: "?".into(), modifier: Some(":-default".into()) },
    ],
    special_variable_pid_with_modifier: "${$:-0}" => [
        TokenKind::Variable { name: "$".into(), modifier: Some(":-0".into()) },
    ],
    special_variable_in_command: "echo $?" => [
        TokenKind::Word("echo".into()),
        TokenKind::Variable { name: "?".into(), modifier: None },
    ],
    special_variable_followed_by_text: "$?foo" => [
        TokenKind::Variable { name: "?".into(), modifier: None },
        TokenKind::Word("foo".into()),
    ],
    consecutive_special_variables: "$?$$" => [
        TokenKind::Variable { name: "?".into(), modifier: None },
        TokenKind::Variable { name: "$".into(), modifier: None },
    ],
}

span_tests! {
    special_variable_exit_code_span: "$?" => [(0, 2)],
    special_variable_pid_span: "$$" => [(0, 2)],
    special_variable_arg_count_span: "$#" => [(0, 2)],
    special_variable_script_name_span: "$0" => [(0, 2)],
}
