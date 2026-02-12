// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Token types for the shell lexer.

use std::fmt;

pub use crate::span::{context_snippet, diagnostic_context, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Word(String),
    /// Single-quoted: literal content, no escape processing or expansion.
    SingleQuoted(String),
    /// Double-quoted: supports escape processing (`\\`, `\n`, `\t`, `\"`, `\'`)
    /// and variable/command expansion via separate word parts.
    DoubleQuoted(Vec<super::ast::WordPart>),
    Variable {
        name: String,
        /// Optional modifier (e.g., `:-default`, `:=value`).
        modifier: Option<String>,
    },
    /// `&&`
    And,
    /// `||`
    Or,
    /// `|`
    Pipe,
    /// `;`
    Semi,
    /// `&`
    Ampersand,
    Newline,
    /// `$(cmd)` or `` `cmd` ``.
    CommandSubstitution {
        content: String,
        backtick: bool,
    },

    /// `>` or `n>`
    RedirectOut {
        fd: Option<u32>,
    },

    /// `>>` or `n>>`
    RedirectAppend {
        fd: Option<u32>,
    },

    /// `<` or `n<`
    RedirectIn {
        fd: Option<u32>,
    },

    /// `<<` or `<<-`
    HereDoc {
        fd: Option<u32>,
        strip_tabs: bool,
        delimiter: String,
        body: String,
        /// Quoted delimiters (`<<'EOF'`) disable variable expansion in the body.
        quoted: bool,
    },

    /// `<<<`
    HereString {
        fd: Option<u32>,
    },

    /// `&>` or `&>>`
    RedirectBoth {
        append: bool,
    },

    /// `n>&m` or `n<&m`
    DuplicateFd {
        source: u32,
        target: DupTarget,
        /// True for output (`>&`), false for input (`<&`).
        output: bool,
    },

    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `{`
    LBrace,
    /// `}`
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

pub(crate) fn is_special_variable(ch: char) -> bool {
    matches!(ch, '?' | '$' | '#' | '0')
}

pub(crate) fn is_valid_variable_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

pub(crate) fn is_valid_variable_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

/// Variable names start with `[a-zA-Z_]` and contain only `[a-zA-Z0-9_]`.
pub(crate) fn is_valid_variable_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_valid_variable_start(c) => {}
        _ => return false,
    }
    chars.all(is_valid_variable_char)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DupTarget {
    Fd(u32),
    /// Close the file descriptor (`>&-`).
    Close,
}

#[cfg(test)]
#[path = "token_tests.rs"]
mod tests;
