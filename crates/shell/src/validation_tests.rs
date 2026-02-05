// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn validation_error_span_returns_correct_span() {
    let span = Span::new(5, 10);

    let cases = [
        ValidationError::EmptyCommand { span },
        ValidationError::MissingCommandAfter {
            operator: "&&".to_string(),
            span,
        },
        ValidationError::MissingCommandBefore {
            operator: "||".to_string(),
            span,
        },
        ValidationError::EmptyJobSegment { span },
        ValidationError::EmptySubshell { span },
        ValidationError::EmptyBraceGroup { span },
        ValidationError::StandaloneAssignment {
            name: "FOO".to_string(),
            value: Some("bar".to_string()),
            span,
        },
        ValidationError::RedirectionWithoutCommand { span },
        ValidationError::ExcessiveNesting {
            depth: 10,
            max: 5,
            span,
        },
    ];

    for error in &cases {
        assert_eq!(error.span(), span, "error variant: {error:?}");
    }
}

#[test]
fn validation_error_display_messages() {
    let span = Span::new(0, 5);

    assert_eq!(
        ValidationError::EmptyCommand { span }.to_string(),
        "empty command at position 0"
    );

    assert_eq!(
        ValidationError::MissingCommandAfter {
            operator: "&&".to_string(),
            span,
        }
        .to_string(),
        "missing command after `&&`"
    );

    assert_eq!(
        ValidationError::EmptySubshell { span }.to_string(),
        "empty subshell"
    );

    assert_eq!(
        ValidationError::EmptyBraceGroup { span }.to_string(),
        "empty brace group"
    );

    assert_eq!(
        ValidationError::StandaloneAssignment {
            name: "FOO".to_string(),
            value: Some("bar".to_string()),
            span,
        }
        .to_string(),
        "assignment without command: `FOO=bar`"
    );

    assert_eq!(
        ValidationError::StandaloneAssignment {
            name: "FOO".to_string(),
            value: None,
            span,
        }
        .to_string(),
        "assignment without command: `FOO=`"
    );

    assert_eq!(
        ValidationError::ExcessiveNesting {
            depth: 10,
            max: 5,
            span,
        }
        .to_string(),
        "excessive nesting depth (10 levels, max 5)"
    );
}

#[test]
fn validation_error_context_shows_location() {
    let input = "echo hello; ( )";
    let span = Span::new(12, 15); // The "( )" part
    let error = ValidationError::EmptySubshell { span };

    let context = error.context(input, 30);
    assert!(
        context.contains("( )"),
        "context should contain the span text"
    );
    assert!(
        context.contains("^^^"),
        "context should contain caret markers"
    );
}
