// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Variable lexer tests: simple variables, braced variables, modifiers, error cases.

use crate::lexer::{Lexer, LexerError};
use crate::token::{Span, TokenKind};

// Variable tests - Simple Variables

#[test]
fn test_simple_variable() {
    let tokens = Lexer::tokenize("$HOME").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "HOME".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[0].span, Span::new(0, 5));
}

#[test]
fn test_variable_with_underscore() {
    let tokens = Lexer::tokenize("$MY_VAR").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "MY_VAR".into(),
            modifier: None,
        }
    );
}

#[test]
fn test_variable_starting_with_underscore() {
    let tokens = Lexer::tokenize("$_private").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "_private".into(),
            modifier: None,
        }
    );
}

#[test]
fn test_variable_with_numbers() {
    let tokens = Lexer::tokenize("$VAR123").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR123".into(),
            modifier: None,
        }
    );
}

#[test]
fn test_var_name_vs_var_hyphen_name() {
    // Underscore is part of variable name
    let tokens = Lexer::tokenize("$VAR_NAME").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR_NAME".into(),
            modifier: None,
        }
    );

    // Hyphen terminates variable name
    let tokens = Lexer::tokenize("$VAR-NAME").unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[1].kind, TokenKind::Word("-NAME".into()));
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
        assert_eq!(
            tokens[0].kind,
            TokenKind::Variable {
                name: var_name.into(),
                modifier: None,
            }
        );
        assert_eq!(tokens[1].kind, TokenKind::Word(word.into()));
    }
}

#[test]
fn test_consecutive_variables() {
    let tokens = Lexer::tokenize("$A$B$C").unwrap();
    assert_eq!(tokens.len(), 3);
    for (i, name) in ["A", "B", "C"].iter().enumerate() {
        assert_eq!(
            tokens[i].kind,
            TokenKind::Variable {
                name: (*name).into(),
                modifier: None,
            }
        );
    }
}

#[test]
fn test_variable_followed_by_and() {
    let tokens = Lexer::tokenize("$VAR&&").unwrap();
    assert_eq!(tokens.len(), 2);
    assert!(matches!(tokens[0].kind, TokenKind::Variable { .. }));
    assert_eq!(tokens[1].kind, TokenKind::And);
}

// Variable tests - Braced Variables

#[test]
fn test_braced_variable() {
    let tokens = Lexer::tokenize("${HOME}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "HOME".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[0].span, Span::new(0, 7));
}

#[test]
fn test_braced_variable_adjacent_to_text() {
    let tokens = Lexer::tokenize("${HOME}/bin").unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "HOME".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[1].kind, TokenKind::Word("/bin".into()));
}

#[test]
fn test_braced_variable_adjacent_text_both_sides() {
    let tokens = Lexer::tokenize("x${VAR}y").unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0].kind, TokenKind::Word("x".into()));
    assert_eq!(
        tokens[1].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[2].kind, TokenKind::Word("y".into()));
}

#[test]
fn test_alternating_braced_vars_and_text() {
    let tokens = Lexer::tokenize("${A}B${C}D").unwrap();
    assert_eq!(tokens.len(), 4);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "A".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[1].kind, TokenKind::Word("B".into()));
    assert_eq!(
        tokens[2].kind,
        TokenKind::Variable {
            name: "C".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[3].kind, TokenKind::Word("D".into()));
}

#[test]
fn test_two_consecutive_braced_vars() {
    let tokens = Lexer::tokenize("${VAR}${OTHER}").unwrap();
    assert_eq!(tokens.len(), 2);
    assert!(matches!(tokens[0].kind, TokenKind::Variable { .. }));
    assert!(matches!(tokens[1].kind, TokenKind::Variable { .. }));
}

#[test]
fn test_braced_variable_span_accuracy() {
    let tokens = Lexer::tokenize("${HOME}").unwrap();
    assert_eq!(tokens[0].span, Span::new(0, 7)); // $ { H O M E }
}

// Variable tests - Variables with Modifiers

#[test]
fn test_variable_default_value() {
    let tokens = Lexer::tokenize("${HOME:-/tmp}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "HOME".into(),
            modifier: Some(":-/tmp".into()),
        }
    );
}

#[test]
fn test_variable_assign_default() {
    let tokens = Lexer::tokenize("${VAR:=default}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":=default".into()),
        }
    );
}

#[test]
fn test_variable_nested_in_modifier() {
    let tokens = Lexer::tokenize("${VAR:-${OTHER}}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":-${OTHER}".into()),
        }
    );
}

#[test]
fn test_variable_use_alternative() {
    let tokens = Lexer::tokenize("${VAR:+value}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":+value".into()),
        }
    );
}

#[test]
fn test_deeply_nested_defaults() {
    let tokens = Lexer::tokenize("${A:-${B:-${C}}}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "A".into(),
            modifier: Some(":-${B:-${C}}".into()),
        }
    );
}

#[test]
fn test_variable_error_if_unset() {
    let tokens = Lexer::tokenize("${VAR:?error message}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":?error message".into()),
        }
    );
}

#[test]
fn test_multiple_nested_vars_in_modifier() {
    let tokens = Lexer::tokenize("${VAR:-${X}${Y}}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":-${X}${Y}".into()),
        }
    );
}

#[test]
fn test_variable_prefix_removal() {
    let tokens = Lexer::tokenize("${VAR#pattern}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some("#pattern".into()),
        }
    );
}

#[test]
fn test_literal_braces_in_modifier() {
    // Literal braces that aren't ${...} should still be counted for depth
    let tokens = Lexer::tokenize("${VAR:-{literal}}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":-{literal}".into()),
        }
    );
}

#[test]
fn test_variable_suffix_removal() {
    let tokens = Lexer::tokenize("${VAR%pattern}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some("%pattern".into()),
        }
    );
}

#[test]
fn test_modifier_followed_by_text() {
    // The first } at depth 0 closes the variable
    let tokens = Lexer::tokenize("${VAR:-a}rest").unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":-a".into()),
        }
    );
    assert_eq!(tokens[1].kind, TokenKind::Word("rest".into()));
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
            TokenKind::Variable {
                name: "VAR".into(),
                modifier: Some(expected_mod.into()),
            }
        );
    }
}

// Variable tests - Error Cases

#[test]
fn test_empty_variable_dollar_only() {
    let result = Lexer::tokenize("$");
    assert!(matches!(
        result,
        Err(LexerError::EmptyVariable { span }) if span == Span::new(0, 1)
    ));
}

#[test]
fn test_empty_variable_dollar_space() {
    let result = Lexer::tokenize("$ ");
    assert!(matches!(result, Err(LexerError::EmptyVariable { .. })));
}

#[test]
fn test_empty_variable_operator() {
    let result = Lexer::tokenize("$&&");
    assert!(matches!(result, Err(LexerError::EmptyVariable { .. })));
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
fn test_unterminated_brace_eof() {
    let result = Lexer::tokenize("${VAR");
    assert!(matches!(
        result,
        Err(LexerError::UnterminatedVariable { .. })
    ));
}

#[test]
fn test_unterminated_modifier() {
    let result = Lexer::tokenize("${VAR:-default");
    assert!(matches!(
        result,
        Err(LexerError::UnterminatedVariable { .. })
    ));
}

#[test]
fn test_unterminated_nested() {
    let result = Lexer::tokenize("${VAR:-${OTHER}");
    assert!(matches!(
        result,
        Err(LexerError::UnterminatedVariable { .. })
    ));
}

#[test]
fn test_invalid_variable_starts_with_number() {
    let result = Lexer::tokenize("${123}");
    assert!(matches!(
        result,
        Err(LexerError::InvalidVariableName { .. })
    ));
}

#[test]
fn test_invalid_name_starts_with_hyphen() {
    let result = Lexer::tokenize("${-name}");
    assert!(matches!(
        result,
        Err(LexerError::InvalidVariableName { .. })
    ));
}

#[test]
fn test_invalid_name_unicode() {
    let result = Lexer::tokenize("${日本}");
    assert!(matches!(
        result,
        Err(LexerError::InvalidVariableName { .. })
    ));
}

#[test]
fn test_error_dollar_in_command() {
    // Error should only affect the problematic variable
    let result = Lexer::tokenize("echo $");
    assert!(matches!(result, Err(LexerError::EmptyVariable { .. })));
}

#[test]
fn test_error_span_accuracy_unterminated() {
    let result = Lexer::tokenize("${LONGNAME");
    if let Err(LexerError::UnterminatedVariable { span }) = result {
        // Span should cover from $ to end of input
        assert_eq!(span.start, 0);
        assert!(span.end >= 10); // At least to end of "LONGNAME"
    } else {
        panic!("Expected UnterminatedVariable error");
    }
}

// Variable tests - Integration with Other Tokens

#[test]
fn test_variable_in_command() {
    let tokens = Lexer::tokenize("echo $HOME").unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].kind, TokenKind::Word("echo".into()));
    assert_eq!(
        tokens[1].kind,
        TokenKind::Variable {
            name: "HOME".into(),
            modifier: None,
        }
    );
}

#[test]
fn test_variable_with_operators() {
    let tokens = Lexer::tokenize("$A && $B").unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "A".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[1].kind, TokenKind::And);
    assert_eq!(
        tokens[2].kind,
        TokenKind::Variable {
            name: "B".into(),
            modifier: None,
        }
    );
}

#[test]
fn test_variable_with_pipe() {
    let tokens = Lexer::tokenize("echo $VAR | cat").unwrap();
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[0].kind, TokenKind::Word("echo".into()));
    assert!(matches!(tokens[1].kind, TokenKind::Variable { .. }));
    assert_eq!(tokens[2].kind, TokenKind::Pipe);
    assert_eq!(tokens[3].kind, TokenKind::Word("cat".into()));
}

#[test]
fn test_variable_with_semicolon() {
    let tokens = Lexer::tokenize("$A; $B").unwrap();
    assert_eq!(tokens.len(), 3);
    assert!(matches!(tokens[0].kind, TokenKind::Variable { .. }));
    assert_eq!(tokens[1].kind, TokenKind::Semi);
    assert!(matches!(tokens[2].kind, TokenKind::Variable { .. }));
}

#[test]
fn test_variable_with_newline() {
    let tokens = Lexer::tokenize("$A\n$B").unwrap();
    assert_eq!(tokens.len(), 3);
    assert!(matches!(tokens[0].kind, TokenKind::Variable { .. }));
    assert_eq!(tokens[1].kind, TokenKind::Newline);
    assert!(matches!(tokens[2].kind, TokenKind::Variable { .. }));
}

#[test]
fn test_variable_after_pipe() {
    let tokens = Lexer::tokenize("cmd | $PAGER").unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0].kind, TokenKind::Word("cmd".into()));
    assert_eq!(tokens[1].kind, TokenKind::Pipe);
    assert_eq!(
        tokens[2].kind,
        TokenKind::Variable {
            name: "PAGER".into(),
            modifier: None,
        }
    );
}

// Variable tests - Edge Cases

#[test]
fn test_simple_variable_followed_by_slash() {
    // $HOME/bin should be Variable + Word
    let tokens = Lexer::tokenize("$HOME/bin").unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "HOME".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[1].kind, TokenKind::Word("/bin".into()));
}

#[test]
fn test_variable_followed_by_dot() {
    let tokens = Lexer::tokenize("$FILE.txt").unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "FILE".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[1].kind, TokenKind::Word(".txt".into()));
}

#[test]
fn test_braced_variable_in_path() {
    let tokens = Lexer::tokenize("/usr/${LOCAL}/bin").unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0].kind, TokenKind::Word("/usr/".into()));
    assert_eq!(
        tokens[1].kind,
        TokenKind::Variable {
            name: "LOCAL".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[2].kind, TokenKind::Word("/bin".into()));
}

#[test]
fn test_variable_with_deeply_nested_braces() {
    let tokens = Lexer::tokenize("${VAR:-${A:-${B}}}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":-${A:-${B}}".into()),
        }
    );
}

#[test]
fn test_unterminated_brace_after_dollar() {
    let result = Lexer::tokenize("${");
    assert!(matches!(
        result,
        Err(LexerError::UnterminatedVariable { .. })
    ));
}

// =============================================================================
// Extreme variable nesting stress tests (Step 2)
// =============================================================================

#[test]
fn test_variable_nesting_depth_20() {
    // ${A:-${B:-${C:-...}}} with 20 levels
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
    // 50 levels of variable nesting
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

// =============================================================================
// AST content preservation tests (Step 4)
// =============================================================================

#[test]
fn test_ast_preserves_nested_variable_verbatim() {
    // Verify nested ${} in modifier is stored as raw string, not recursively parsed
    let tokens = Lexer::tokenize("${A:-${B}}").unwrap();
    assert_eq!(tokens.len(), 1);
    if let TokenKind::Variable { name, modifier } = &tokens[0].kind {
        assert_eq!(name, "A");
        assert_eq!(modifier.as_ref().unwrap(), ":-${B}");
        // The inner ${B} is NOT parsed - it's just part of the modifier string
    } else {
        panic!("Expected Variable token");
    }
}

#[test]
fn test_ast_preserves_deeply_nested_variable() {
    // Three levels of variable nesting
    let tokens = Lexer::tokenize("${A:-${B:-${C}}}").unwrap();
    assert_eq!(tokens.len(), 1);
    if let TokenKind::Variable { name, modifier } = &tokens[0].kind {
        assert_eq!(name, "A");
        assert_eq!(modifier.as_ref().unwrap(), ":-${B:-${C}}");
        // Modifier contains literal ":-${B:-${C}}" - NOT parsed to AST
    } else {
        panic!("Expected Variable token");
    }
}

// =============================================================================
// Unbalanced grouping error handling tests (Step 5)
// =============================================================================

#[test]
fn test_error_nested_variable_unterminated() {
    // ${A:-${B:-${C}  <- missing outer }}
    let result = Lexer::tokenize("${A:-${B:-${C}");
    assert!(
        matches!(result, Err(LexerError::UnterminatedVariable { .. })),
        "Expected UnterminatedVariable, got: {:?}",
        result
    );
}

#[test]
fn test_error_variable_span_points_to_outermost() {
    // Verify span starts at the outermost ${ that's unterminated
    let result = Lexer::tokenize("${A:-${B:-${C}");
    if let Err(LexerError::UnterminatedVariable { span }) = result {
        assert_eq!(span.start, 0, "Span should point to outermost ${{");
    } else {
        panic!("Expected UnterminatedVariable error, got: {:?}", result);
    }
}

#[test]
fn test_error_literal_braces_unbalanced() {
    // Literal { in modifier without matching } should still be tracked
    let result = Lexer::tokenize("${VAR:-{incomplete");
    assert!(
        matches!(result, Err(LexerError::UnterminatedVariable { .. })),
        "Expected UnterminatedVariable, got: {:?}",
        result
    );
}

// =============================================================================
// Special shell variables ($?, $$, $#, $0)
// =============================================================================

#[test]
fn test_special_variable_exit_code() {
    let tokens = Lexer::tokenize("$?").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "?".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[0].span, Span::new(0, 2));
}

#[test]
fn test_special_variable_pid() {
    let tokens = Lexer::tokenize("$$").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "$".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[0].span, Span::new(0, 2));
}

#[test]
fn test_special_variable_arg_count() {
    let tokens = Lexer::tokenize("$#").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "#".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[0].span, Span::new(0, 2));
}

#[test]
fn test_special_variable_script_name() {
    let tokens = Lexer::tokenize("$0").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "0".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[0].span, Span::new(0, 2));
}

#[test]
fn test_special_variable_braced_exit_code() {
    let tokens = Lexer::tokenize("${?}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "?".into(),
            modifier: None,
        }
    );
}

#[test]
fn test_special_variable_braced_pid() {
    let tokens = Lexer::tokenize("${$}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "$".into(),
            modifier: None,
        }
    );
}

#[test]
fn test_special_variable_braced_arg_count() {
    let tokens = Lexer::tokenize("${#}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "#".into(),
            modifier: None,
        }
    );
}

#[test]
fn test_special_variable_braced_script_name() {
    let tokens = Lexer::tokenize("${0}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "0".into(),
            modifier: None,
        }
    );
}

#[test]
fn test_special_variable_with_modifier() {
    let tokens = Lexer::tokenize("${?:-default}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "?".into(),
            modifier: Some(":-default".into()),
        }
    );
}

#[test]
fn test_special_variable_pid_with_modifier() {
    let tokens = Lexer::tokenize("${$:-0}").unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "$".into(),
            modifier: Some(":-0".into()),
        }
    );
}

#[test]
fn test_special_variable_in_command() {
    let tokens = Lexer::tokenize("echo $?").unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].kind, TokenKind::Word("echo".into()));
    assert_eq!(
        tokens[1].kind,
        TokenKind::Variable {
            name: "?".into(),
            modifier: None,
        }
    );
}

#[test]
fn test_special_variable_followed_by_text() {
    let tokens = Lexer::tokenize("$?foo").unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "?".into(),
            modifier: None,
        }
    );
    assert_eq!(tokens[1].kind, TokenKind::Word("foo".into()));
}

#[test]
fn test_consecutive_special_variables() {
    let tokens = Lexer::tokenize("$?$$").unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(
        tokens[0].kind,
        TokenKind::Variable {
            name: "?".into(),
            modifier: None,
        }
    );
    assert_eq!(
        tokens[1].kind,
        TokenKind::Variable {
            name: "$".into(),
            modifier: None,
        }
    );
}
