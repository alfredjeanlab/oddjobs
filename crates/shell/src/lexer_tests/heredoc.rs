// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Here-document lexer tests: delimiter capture and body reading.

use crate::lexer::{Lexer, LexerError};
use crate::token::TokenKind;

// =============================================================================
// Basic Heredoc with Body
// =============================================================================

lex_tests! {
    basic_heredoc: "cat <<EOF\nhello\nworld\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "hello\nworld\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    single_line_body: "cat <<EOF\nhello\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "hello\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    empty_body: "cat <<EOF\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    custom_delimiter: "cat <<MARKER\ndata\nMARKER" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "MARKER".into(),
            body: "data\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
}

// =============================================================================
// Tab Stripping (<<-)
// =============================================================================

lex_tests! {
    strip_tabs_body: "cat <<-EOF\n\thello\n\tworld\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: true,
            delimiter: "EOF".into(),
            body: "hello\nworld\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    strip_tabs_delimiter: "cat <<-EOF\n\thello\n\tEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: true,
            delimiter: "EOF".into(),
            body: "hello\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    strip_multiple_tabs: "cat <<-EOF\n\t\thello\n\t\t\tworld\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: true,
            delimiter: "EOF".into(),
            body: "hello\nworld\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    strip_tabs_mixed: "cat <<-EOF\n\thello\nno tabs\n\ttabbed\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: true,
            delimiter: "EOF".into(),
            body: "hello\nno tabs\ntabbed\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
}

// =============================================================================
// File Descriptor Prefix
// =============================================================================

lex_tests! {
    heredoc_fd_3: "cmd 3<<EOF\ncontent\nEOF" => [
        TokenKind::Word("cmd".into()),
        TokenKind::HereDoc {
            fd: Some(3),
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "content\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    heredoc_fd_0: "cmd 0<<EOF\ncontent\nEOF" => [
        TokenKind::Word("cmd".into()),
        TokenKind::HereDoc {
            fd: Some(0),
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "content\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    heredoc_fd_strip_tabs: "cmd 3<<-EOF\n\tcontent\n\tEOF" => [
        TokenKind::Word("cmd".into()),
        TokenKind::HereDoc {
            fd: Some(3),
            strip_tabs: true,
            delimiter: "EOF".into(),
            body: "content\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
}

// =============================================================================
// Quoted Delimiters
// =============================================================================

lex_tests! {
    single_quoted_delimiter: "cat <<'EOF'\nhello\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "hello\n".into(),
            quoted: true,
        },
        TokenKind::Newline,
    ],
    double_quoted_delimiter: "cat <<\"EOF\"\nhello\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "hello\n".into(),
            quoted: true,
        },
        TokenKind::Newline,
    ],
    quoted_delimiter_with_space: "cat <<'END OF FILE'\nhello\nEND OF FILE" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "END OF FILE".into(),
            body: "hello\n".into(),
            quoted: true,
        },
        TokenKind::Newline,
    ],
}

// =============================================================================
// Heredoc in Pipeline
// =============================================================================

lex_tests! {
    heredoc_pipe: "cat <<EOF | sort\ndata\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "data\n".into(),
            quoted: false,
        },
        TokenKind::Pipe,
        TokenKind::Word("sort".into()),
        TokenKind::Newline,
    ],
    heredoc_pipe_chain: "cat <<EOF | sort | uniq\ndata\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "data\n".into(),
            quoted: false,
        },
        TokenKind::Pipe,
        TokenKind::Word("sort".into()),
        TokenKind::Pipe,
        TokenKind::Word("uniq".into()),
        TokenKind::Newline,
    ],
}

// =============================================================================
// Multiple Heredocs
// =============================================================================

lex_tests! {
    two_heredocs: "cat <<A <<B\na\nA\nb\nB" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "A".into(),
            body: "a\n".into(),
            quoted: false,
        },
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "B".into(),
            body: "b\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
}

// =============================================================================
// Body Content Preservation
// =============================================================================

lex_tests! {
    body_with_quotes: "cat <<EOF\n\"quoted\"\n'single'\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "\"quoted\"\n'single'\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    body_with_variables: "cat <<EOF\n$VAR ${VAR}\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "$VAR ${VAR}\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    body_apparent_delimiter: "cat <<EOF\nnot EOF here\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "not EOF here\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
    body_with_spaces: "cat <<EOF\n  indented\n    more\nEOF" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereDoc {
            fd: None,
            strip_tabs: false,
            delimiter: "EOF".into(),
            body: "  indented\n    more\n".into(),
            quoted: false,
        },
        TokenKind::Newline,
    ],
}

// =============================================================================
// Error Cases
// =============================================================================

lex_error_tests! {
    unterminated_heredoc: "cat <<EOF\nhello" => LexerError::UnterminatedHereDoc { .. },
    unterminated_no_body: "cat <<EOF" => LexerError::UnterminatedHereDoc { .. },
    missing_delimiter: "cat <<\nEOF" => LexerError::InvalidRedirection { .. },
    missing_delimiter_strip: "cat <<-\nEOF" => LexerError::InvalidRedirection { .. },
}

// =============================================================================
// Span Tests
// =============================================================================

// HereDoc span covers the operator and delimiter on the command line (not the body).
// The body content is captured but doesn't extend the span.
span_tests! {
    heredoc_span: "cat <<EOF\nhello\nEOF" => [(0, 3), (4, 9), (9, 10)],
    heredoc_strip_span: "cat <<-EOF\nhello\n\tEOF" => [(0, 3), (4, 10), (10, 11)],
}
