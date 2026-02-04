// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::Parser;

fn validate_str(input: &str) -> Result<(), Vec<ValidationError>> {
    let ast = Parser::parse(input).expect("parse failed");
    validate(&ast)
}

fn validate_str_with_config(
    input: &str,
    config: ValidatorConfig,
) -> Result<(), Vec<ValidationError>> {
    let ast = Parser::parse(input).expect("parse failed");
    validate_with_config(&ast, config)
}

// =============================================================================
// Valid Cases
// =============================================================================

#[test]
fn valid_simple_command() {
    assert!(validate_str("echo hello").is_ok());
}

#[test]
fn valid_command_with_args() {
    assert!(validate_str("ls -la /tmp").is_ok());
}

#[test]
fn valid_pipeline() {
    assert!(validate_str("cat file | grep pattern | wc -l").is_ok());
}

#[test]
fn valid_and_or_chain() {
    assert!(validate_str("true && echo yes || echo no").is_ok());
}

#[test]
fn valid_subshell() {
    assert!(validate_str("(cd /tmp && ls)").is_ok());
}

#[test]
fn valid_brace_group() {
    assert!(validate_str("{ echo a; echo b; }").is_ok());
}

#[test]
fn valid_nested_subshell() {
    assert!(validate_str("(echo outer; (echo inner))").is_ok());
}

#[test]
fn valid_command_substitution() {
    assert!(validate_str("echo $(date)").is_ok());
}

#[test]
fn valid_variable_reference() {
    assert!(validate_str("echo $HOME").is_ok());
}

#[test]
fn valid_background_command() {
    assert!(validate_str("sleep 10 &").is_ok());
}

#[test]
fn valid_multiple_commands() {
    assert!(validate_str("echo a; echo b; echo c").is_ok());
}

#[test]
fn valid_empty_input() {
    assert!(validate_str("").is_ok());
}

#[test]
fn valid_assignment_with_command() {
    assert!(validate_str("FOO=bar echo $FOO").is_ok());
}

// =============================================================================
// Error Cases - Empty Structures
// =============================================================================

#[test]
fn error_empty_subshell() {
    let err = validate_str("( )").unwrap_err();
    assert_eq!(err.len(), 1);
    assert!(matches!(err[0], ValidationError::EmptySubshell { .. }));
}

#[test]
fn error_empty_brace_group() {
    let err = validate_str("{ }").unwrap_err();
    assert_eq!(err.len(), 1);
    assert!(matches!(err[0], ValidationError::EmptyBraceGroup { .. }));
}

#[test]
fn error_empty_subshell_nested() {
    let err = validate_str("(echo before; ( ); echo after)").unwrap_err();
    assert_eq!(err.len(), 1);
    assert!(matches!(err[0], ValidationError::EmptySubshell { .. }));
}

// =============================================================================
// IFS Assignment Validation
// =============================================================================

#[test]
fn error_ifs_assignment_with_command() {
    let err = validate_str("IFS=: read -r a b").unwrap_err();
    assert_eq!(err.len(), 1);
    assert!(matches!(err[0], ValidationError::IfsAssignment { .. }));
}

#[test]
fn error_ifs_assignment_in_pipeline() {
    let err = validate_str("echo data | IFS=, read -r a b").unwrap_err();
    assert_eq!(err.len(), 1);
    assert!(matches!(err[0], ValidationError::IfsAssignment { .. }));
}

#[test]
fn error_ifs_assignment_error_message() {
    let err = validate_str("IFS=: read -r a b").unwrap_err();
    let msg = err[0].to_string();
    assert!(msg.contains("IFS configuration is not supported"));
    assert!(msg.contains("word splitting uses default whitespace"));
}

#[test]
fn ok_variable_named_like_ifs_prefix() {
    // IFS_BACKUP is fine, only exactly "IFS" is rejected
    assert!(validate_str("IFS_BACKUP=x echo test").is_ok());
}

// =============================================================================
// Error Cases - Excessive Nesting
// =============================================================================

#[test]
fn error_excessive_nesting_subshells() {
    let config = ValidatorConfig {
        max_nesting_depth: 3,
        ..Default::default()
    };
    let input = "((((echo too deep))))";
    let err = validate_str_with_config(input, config).unwrap_err();
    assert!(err.iter().any(|e| matches!(
        e,
        ValidationError::ExcessiveNesting {
            depth: 4,
            max: 3,
            ..
        }
    )));
}

#[test]
fn error_excessive_nesting_brace_groups() {
    let config = ValidatorConfig {
        max_nesting_depth: 2,
        ..Default::default()
    };
    let input = "{ { { echo deep; }; }; }";
    let err = validate_str_with_config(input, config).unwrap_err();
    assert!(err.iter().any(|e| matches!(
        e,
        ValidationError::ExcessiveNesting {
            depth: 3,
            max: 2,
            ..
        }
    )));
}

#[test]
fn error_excessive_nesting_mixed() {
    let config = ValidatorConfig {
        max_nesting_depth: 2,
        ..Default::default()
    };
    let input = "({ { echo deep; }; })";
    let err = validate_str_with_config(input, config).unwrap_err();
    assert!(err
        .iter()
        .any(|e| matches!(e, ValidationError::ExcessiveNesting { .. })));
}

#[test]
fn ok_at_max_nesting_depth() {
    let config = ValidatorConfig {
        max_nesting_depth: 3,
        ..Default::default()
    };
    // Exactly 3 levels should be OK
    let input = "(((echo ok)))";
    assert!(validate_str_with_config(input, config).is_ok());
}

#[test]
fn unlimited_nesting_when_zero() {
    let config = ValidatorConfig {
        max_nesting_depth: 0, // unlimited
        ..Default::default()
    };
    let input = "((((((((((echo deep))))))))))";
    assert!(validate_str_with_config(input, config).is_ok());
}

// =============================================================================
// Multiple Errors
// =============================================================================

#[test]
fn collects_multiple_errors() {
    let err = validate_str("( ); { }").unwrap_err();
    assert_eq!(err.len(), 2);
    assert!(err
        .iter()
        .any(|e| matches!(e, ValidationError::EmptySubshell { .. })));
    assert!(err
        .iter()
        .any(|e| matches!(e, ValidationError::EmptyBraceGroup { .. })));
}

#[test]
fn collects_errors_at_different_depths() {
    let err = validate_str("(( ); echo ok)").unwrap_err();
    assert!(!err.is_empty());
    assert!(err
        .iter()
        .any(|e| matches!(e, ValidationError::EmptySubshell { .. })));
}

// =============================================================================
// Error Span Verification
// =============================================================================

#[test]
fn error_span_points_to_subshell() {
    let input = "echo hello; ( )";
    let err = validate_str(input).unwrap_err();
    assert_eq!(err.len(), 1);
    let span = err[0].span();
    let slice = span.slice(input);
    assert_eq!(slice, "( )");
}

#[test]
fn error_span_points_to_brace_group() {
    let input = "{ }";
    let err = validate_str(input).unwrap_err();
    assert_eq!(err.len(), 1);
    let span = err[0].span();
    let slice = span.slice(input);
    assert_eq!(slice, "{ }");
}

// =============================================================================
// Error Context
// =============================================================================

#[test]
fn error_context_shows_location() {
    let input = "echo hello; ( )";
    let err = validate_str(input).unwrap_err();
    let ctx = err[0].context(input, 30);
    assert!(ctx.contains("( )"));
    assert!(ctx.contains("^^^"));
}

// =============================================================================
// Command Substitution Validation
// =============================================================================

#[test]
fn validates_inside_command_substitution() {
    let input = "echo $(( ))";
    let err = validate_str(input).unwrap_err();
    assert!(err
        .iter()
        .any(|e| matches!(e, ValidationError::EmptySubshell { .. })));
}

// =============================================================================
// Property-Based Tests
// =============================================================================

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn valid_commands_validate(cmd in "[a-z]+ [a-z]+") {
            // Simple valid commands should always pass
            if let Ok(ast) = Parser::parse(&cmd) {
                prop_assert!(validate(&ast).is_ok());
            }
        }

        #[test]
        fn validation_never_panics(input in "[ -~]{0,100}") {
            // Validator should never panic, even on invalid input
            if let Ok(ast) = Parser::parse(&input) {
                let _ = validate(&ast);
            }
        }

        #[test]
        fn errors_have_valid_spans(input in r"\(\s*\)|\{\s*\}") {
            if let Ok(ast) = Parser::parse(&input) {
                if let Err(errors) = validate(&ast) {
                    for err in errors {
                        let span = err.span();
                        prop_assert!(span.start <= input.len());
                        prop_assert!(span.end <= input.len());
                    }
                }
            }
        }

        #[test]
        fn idempotent_validation(input in "[a-z]+ [a-z]*") {
            if let Ok(ast) = Parser::parse(&input) {
                let result1 = validate(&ast);
                let result2 = validate(&ast);
                prop_assert_eq!(result1.is_ok(), result2.is_ok());
            }
        }
    }
}
