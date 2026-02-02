// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Simple operators (&, |, ;, newlines, etc.).

use super::Lexer;
use crate::token::{Span, Token, TokenKind};

impl Lexer<'_> {
    /// Skip consecutive newlines and intervening whitespace.
    pub(super) fn skip_newlines(&mut self) {
        loop {
            match self.peek_char() {
                Some('\n' | '\r') => {
                    self.consume_newline();
                }
                Some(' ' | '\t') => {
                    self.chars.next();
                }
                _ => break,
            }
        }
    }

    /// Lex a newline token, collapsing multiple consecutive newlines.
    ///
    /// Called when peek() has confirmed the next char is '\n'.
    pub(super) fn lex_newline(&mut self, start: usize) -> Token {
        self.chars.next(); // consume \n
        if self.pending_heredocs.is_empty() {
            self.skip_newlines();
        }
        Token::new(TokenKind::Newline, Span::new(start, start + 1))
    }

    /// Lex an ampersand, &&, &>, or &>> operator.
    ///
    /// Called when peek() has confirmed the next char is '&'.
    pub(super) fn lex_ampersand(&mut self, start: usize) -> Token {
        self.chars.next(); // consume &

        match self.peek_char() {
            Some('&') => {
                self.chars.next();
                Token::new(TokenKind::And, Span::new(start, start + 2))
            }
            Some('>') => {
                self.chars.next();
                let append = self.peek_char() == Some('>');
                if append {
                    self.chars.next();
                }
                Token::new(
                    TokenKind::RedirectBoth { append },
                    Span::new(start, start + if append { 3 } else { 2 }),
                )
            }
            _ => Token::new(TokenKind::Ampersand, Span::new(start, start + 1)),
        }
    }

    /// Lex a pipe or || operator.
    ///
    /// Called when peek() has confirmed the next char is '|'.
    pub(super) fn lex_pipe(&mut self, start: usize) -> Token {
        self.chars.next(); // consume |

        if self.peek_char() == Some('|') {
            self.chars.next();
            Token::new(TokenKind::Or, Span::new(start, start + 2))
        } else {
            Token::new(TokenKind::Pipe, Span::new(start, start + 1))
        }
    }
}
