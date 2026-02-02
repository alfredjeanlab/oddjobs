// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn test_span_new() {
    let span = Span::new(0, 5);
    assert_eq!(span.start, 0);
    assert_eq!(span.end, 5);
}

#[test]
fn test_span_len() {
    let span = Span::new(10, 20);
    assert_eq!(span.len(), 10);
}

#[test]
fn test_span_is_empty() {
    let empty = Span::new(5, 5);
    let non_empty = Span::new(5, 10);
    assert!(empty.is_empty());
    assert!(!non_empty.is_empty());
}

#[test]
fn test_span_merge() {
    let a = Span::new(0, 5);
    let b = Span::new(10, 15);
    let merged = a.merge(b);
    assert_eq!(merged.start, 0);
    assert_eq!(merged.end, 15);
}

#[test]
fn test_span_merge_overlapping() {
    let a = Span::new(0, 10);
    let b = Span::new(5, 15);
    let merged = a.merge(b);
    assert_eq!(merged.start, 0);
    assert_eq!(merged.end, 15);
}

#[test]
fn test_span_contains() {
    let span = Span::new(5, 10);
    assert!(!span.contains(4));
    assert!(span.contains(5));
    assert!(span.contains(7));
    assert!(span.contains(9));
    assert!(!span.contains(10));
}

#[test]
fn test_token_new() {
    let token = Token::new(TokenKind::Word("hello".into()), Span::new(0, 5));
    assert_eq!(token.kind, TokenKind::Word("hello".into()));
    assert_eq!(token.span, Span::new(0, 5));
}

#[test]
fn test_token_kind_equality() {
    assert_eq!(TokenKind::And, TokenKind::And);
    assert_eq!(TokenKind::Or, TokenKind::Or);
    assert_eq!(TokenKind::Pipe, TokenKind::Pipe);
    assert_eq!(TokenKind::Semi, TokenKind::Semi);
    assert_eq!(TokenKind::Ampersand, TokenKind::Ampersand);
    assert_eq!(TokenKind::Newline, TokenKind::Newline);
    assert_eq!(
        TokenKind::Word("test".into()),
        TokenKind::Word("test".into())
    );
    assert_ne!(TokenKind::Word("a".into()), TokenKind::Word("b".into()));
    assert_eq!(
        TokenKind::Variable {
            name: "HOME".into(),
            modifier: None
        },
        TokenKind::Variable {
            name: "HOME".into(),
            modifier: None
        }
    );
    assert_eq!(
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":-default".into())
        },
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":-default".into())
        }
    );
    assert_ne!(
        TokenKind::Variable {
            name: "A".into(),
            modifier: None
        },
        TokenKind::Variable {
            name: "B".into(),
            modifier: None
        }
    );
    // CommandSubstitution equality
    assert_eq!(
        TokenKind::CommandSubstitution {
            content: "echo hello".into(),
            backtick: false
        },
        TokenKind::CommandSubstitution {
            content: "echo hello".into(),
            backtick: false
        }
    );
    assert_eq!(
        TokenKind::CommandSubstitution {
            content: "date".into(),
            backtick: true
        },
        TokenKind::CommandSubstitution {
            content: "date".into(),
            backtick: true
        }
    );
    assert_ne!(
        TokenKind::CommandSubstitution {
            content: "a".into(),
            backtick: false
        },
        TokenKind::CommandSubstitution {
            content: "b".into(),
            backtick: false
        }
    );
    assert_ne!(
        TokenKind::CommandSubstitution {
            content: "cmd".into(),
            backtick: false
        },
        TokenKind::CommandSubstitution {
            content: "cmd".into(),
            backtick: true
        }
    );
}

#[test]
fn test_variable_token_equality() {
    assert_eq!(
        TokenKind::Variable {
            name: "HOME".into(),
            modifier: None,
        },
        TokenKind::Variable {
            name: "HOME".into(),
            modifier: None,
        }
    );
    assert_eq!(
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":-default".into()),
        },
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":-default".into()),
        }
    );
    assert_ne!(
        TokenKind::Variable {
            name: "A".into(),
            modifier: None,
        },
        TokenKind::Variable {
            name: "B".into(),
            modifier: None,
        }
    );
    assert_ne!(
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: None,
        },
        TokenKind::Variable {
            name: "VAR".into(),
            modifier: Some(":-x".into()),
        }
    );
}

#[test]
fn test_span_empty() {
    let span = Span::empty(10);
    assert_eq!(span.start, 10);
    assert_eq!(span.end, 10);
    assert!(span.is_empty());
}

#[test]
fn test_span_slice() {
    let source = "hello world";
    let span = Span::new(6, 11);
    assert_eq!(span.slice(source), "world");
}

#[test]
fn test_span_slice_out_of_bounds() {
    let source = "hi";
    let span = Span::new(10, 20);
    // Out of bounds returns empty string
    assert_eq!(span.slice(source), "");
}

#[test]
fn test_span_slice_unicode() {
    let source = "你好世界";
    // "你好" is 6 bytes, "世界" starts at byte 6
    let span = Span::new(0, 6);
    assert_eq!(span.slice(source), "你好");
}
