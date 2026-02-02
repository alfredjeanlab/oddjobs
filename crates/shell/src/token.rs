// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Token types for the shell lexer.

use std::fmt;

pub use crate::span::{context_snippet, diagnostic_context, Span};

/// Token with span information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The kind of token.
    pub kind: TokenKind,
    /// Source location span.
    pub span: Span,
}

impl Token {
    /// Create a new token with the given kind and span.
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Token kinds representing different shell syntax elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// A word (command, argument, etc.)
    Word(String),
    /// Single-quoted string with literal content (no escape processing).
    ///
    /// Examples:
    /// - `'hello'` -> `SingleQuoted("hello")`
    /// - `'$VAR'` -> `SingleQuoted("$VAR")` (not expanded)
    /// - `'it'\''s'` -> Not supported (shell uses quote termination for escaping)
    SingleQuoted(String),
    /// Double-quoted string with escape processing and expansion.
    ///
    /// Processed escapes: `\\`, `\n`, `\t`, `\"`, `\'`
    /// Variable references and command substitutions are parsed into separate
    /// parts for AST analysis.
    ///
    /// Examples:
    /// - `"hello"` -> `DoubleQuoted([Literal("hello")])`
    /// - `"$HOME/bin"` -> `DoubleQuoted([Variable { name: "HOME", .. }, Literal("/bin")])`
    /// - `"$(date)"` -> `DoubleQuoted([CommandSubstitution { content: "date", .. }])`
    DoubleQuoted(Vec<super::ast::WordPart>),
    /// Variable reference with optional modifier.
    ///
    /// Examples:
    /// - `$HOME` -> `Variable { name: "HOME", modifier: None }`
    /// - `${HOME}` -> `Variable { name: "HOME", modifier: None }`
    /// - `${HOME:-/tmp}` -> `Variable { name: "HOME", modifier: Some(":-/tmp") }`
    Variable {
        /// The variable name (without $ or braces).
        name: String,
        /// Optional modifier (e.g., `:-default`, `:=value`).
        modifier: Option<String>,
    },
    /// && - logical AND
    And,
    /// || - logical OR
    Or,
    /// | - pipe
    Pipe,
    /// ; - command separator
    Semi,
    /// & - background
    Ampersand,
    /// Newline (command separator)
    Newline,
    /// Command substitution (`$(cmd)` or `` `cmd` ``).
    ///
    /// Examples:
    /// - `$(echo hello)` -> `CommandSubstitution { content: "echo hello", backtick: false }`
    /// - `` `date` `` -> `CommandSubstitution { content: "date", backtick: true }`
    /// - `$(cat $(file))` -> `CommandSubstitution { content: "cat $(file)", backtick: false }`
    CommandSubstitution {
        /// The command content (without delimiters).
        content: String,
        /// True if backtick syntax was used (legacy).
        backtick: bool,
    },

    /// Output redirection `>` or `n>` where n is a file descriptor.
    RedirectOut {
        /// Source file descriptor (default: 1 for stdout).
        fd: Option<u32>,
    },

    /// Append output redirection `>>` or `n>>`.
    RedirectAppend {
        /// Source file descriptor (default: 1 for stdout).
        fd: Option<u32>,
    },

    /// Input redirection `<` or `n<`.
    RedirectIn {
        /// Target file descriptor (default: 0 for stdin).
        fd: Option<u32>,
    },

    /// Here-document `<<` or `<<-` (with optional dash for tab stripping).
    HereDoc {
        /// Source file descriptor (default: 0 for stdin).
        fd: Option<u32>,
        /// Strip leading tabs from content.
        strip_tabs: bool,
        /// The delimiter word (e.g., "EOF").
        delimiter: String,
        /// The captured body content (lines between heredoc start and delimiter).
        body: String,
        /// Whether the delimiter was quoted (`<<'EOF'` or `<<"EOF"`).
        /// Quoted delimiters disable variable expansion in the body.
        quoted: bool,
    },

    /// Here-string `<<<`.
    HereString {
        /// Source file descriptor (default: 0 for stdin).
        fd: Option<u32>,
    },

    /// Combined stdout/stderr redirection `&>` or `&>>`.
    RedirectBoth {
        /// True for append mode `&>>`, false for `&>`.
        append: bool,
    },

    /// File descriptor duplication `n>&m` or `n<&m`.
    DuplicateFd {
        /// Source file descriptor.
        source: u32,
        /// Target file descriptor or `-` for close.
        target: DupTarget,
        /// Direction: true for output (`>&`), false for input (`<&`).
        output: bool,
    },

    /// `(` - left parenthesis (for subshells).
    LParen,
    /// `)` - right parenthesis.
    RParen,
    /// `{` - left brace (for brace groups).
    LBrace,
    /// `}` - right brace.
    RBrace,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Word(s) => write!(f, "word '{}'", s),
            TokenKind::SingleQuoted(s) => write!(f, "word '{}'", s),
            TokenKind::DoubleQuoted(_) => write!(f, "quoted string"),
            TokenKind::Variable { name, .. } => write!(f, "variable '${}'", name),
            TokenKind::And => write!(f, "'&&'"),
            TokenKind::Or => write!(f, "'||'"),
            TokenKind::Pipe => write!(f, "'|'"),
            TokenKind::Semi => write!(f, "';'"),
            TokenKind::Ampersand => write!(f, "'&'"),
            TokenKind::Newline => write!(f, "newline"),
            TokenKind::CommandSubstitution { .. } => write!(f, "command substitution"),
            TokenKind::RedirectOut { .. } => write!(f, "'>'"),
            TokenKind::RedirectAppend { .. } => write!(f, "'>>'"),
            TokenKind::RedirectIn { .. } => write!(f, "'<'"),
            TokenKind::HereDoc { .. } => write!(f, "'<<'"),
            TokenKind::HereString { .. } => write!(f, "'<<<'"),
            TokenKind::RedirectBoth { append: true } => write!(f, "'&>>'"),
            TokenKind::RedirectBoth { append: false } => write!(f, "'&>'"),
            TokenKind::DuplicateFd { .. } => write!(f, "fd duplication"),
            TokenKind::LParen => write!(f, "'('"),
            TokenKind::RParen => write!(f, "')'"),
            TokenKind::LBrace => write!(f, "'{{'"),
            TokenKind::RBrace => write!(f, "'}}'"),
        }
    }
}

impl TokenKind {
    /// Check if this token kind is a redirection operator.
    pub fn is_redirection(&self) -> bool {
        matches!(
            self,
            TokenKind::RedirectOut { .. }
                | TokenKind::RedirectAppend { .. }
                | TokenKind::RedirectIn { .. }
                | TokenKind::HereDoc { .. }
                | TokenKind::HereString { .. }
                | TokenKind::RedirectBoth { .. }
                | TokenKind::DuplicateFd { .. }
        )
    }
}

/// Check if a character is a special shell variable name.
///
/// Special variables: `?`, `$`, `#`, `0`
pub(crate) fn is_special_variable(ch: char) -> bool {
    matches!(ch, '?' | '$' | '#' | '0')
}

/// Check if a character is a valid start for a variable name.
pub(crate) fn is_valid_variable_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

/// Check if a character is valid within a variable name.
pub(crate) fn is_valid_variable_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Check if a string is a valid shell variable name.
///
/// Variable names start with `[a-zA-Z_]` and contain only `[a-zA-Z0-9_]`.
pub(crate) fn is_valid_variable_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_valid_variable_start(c) => {}
        _ => return false,
    }
    chars.all(is_valid_variable_char)
}

/// Target for file descriptor duplication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DupTarget {
    /// Duplicate to another file descriptor.
    Fd(u32),
    /// Close the file descriptor.
    Close,
}

#[cfg(test)]
#[path = "token_tests.rs"]
mod tests;
