// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Command substitution ($(...) and backticks).

use super::{Lexer, LexerError, QuoteState};
use crate::token::{Span, Token, TokenKind};

impl Lexer<'_> {
    /// Lex a command substitution (`$(cmd)`).
    ///
    /// Called when we've seen `$` and peeked `(`.
    /// Handles quotes inside substitutions so `$(echo ")")` works correctly.
    ///
    /// ## Depth Tracking
    ///
    /// Tracks ALL parentheses for balance, not just `$()` markers:
    /// - Input: `$(echo (a) (b))`
    /// - Depth: 1→2→1→2→1→0
    ///
    /// This is correct because shell command substitution must balance all parentheses.
    /// Content is stored as a raw string (lazy parsing) rather than recursively parsed.
    ///
    /// ## Error Span Strategy
    ///
    /// On unterminated substitution, the span points to the start of the outermost
    /// unclosed `$(`, helping users locate where the construct began.
    pub(super) fn lex_dollar_substitution(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next(); // consume (

        let content = self.read_balanced_content('(', ')', start)?;
        Ok(Token::new(
            TokenKind::CommandSubstitution {
                content,
                backtick: false,
            },
            Span::new(start, self.current_position()),
        ))
    }

    /// Lex a backtick command substitution (`` `cmd` ``).
    ///
    /// Called when peek() has confirmed the next char is '`'.
    pub(super) fn lex_backtick_substitution(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next(); // consume opening `

        let content = self.read_backtick_content(start)?;
        Ok(Token::new(
            TokenKind::CommandSubstitution {
                content,
                backtick: true,
            },
            Span::new(start, self.current_position()),
        ))
    }

    /// Read content until closing backtick.
    ///
    /// Handles escape sequences within backtick content.
    pub(super) fn read_backtick_content(&mut self, start: usize) -> Result<String, LexerError> {
        let content_start = self.current_position();
        let mut content_end = content_start;
        let mut escaped = false;

        while let Some(&(pos, ch)) = self.chars.peek() {
            if escaped {
                escaped = false;
                content_end = pos + ch.len_utf8();
                self.chars.next();
                continue;
            }

            match ch {
                '\\' => {
                    escaped = true;
                    content_end = pos + ch.len_utf8();
                    self.chars.next();
                }
                '`' => {
                    let content = self.input[content_start..content_end].to_string();
                    self.chars.next(); // consume closing `
                    return Ok(content);
                }
                _ => {
                    content_end = pos + ch.len_utf8();
                    self.chars.next();
                }
            }
        }

        Err(LexerError::UnterminatedSubstitution {
            span: Span::new(start, content_end),
        })
    }

    /// Read balanced content until closing delimiter.
    ///
    /// Tracks nested parens/braces and respects quotes.
    pub(super) fn read_balanced_content(
        &mut self,
        open: char,
        close: char,
        start: usize,
    ) -> Result<String, LexerError> {
        let content_start = self.current_position();
        let mut content_end = content_start;
        let mut depth = 1;
        let mut quotes = QuoteState::new();

        while let Some(&(pos, ch)) = self.chars.peek() {
            let is_literal = quotes.process(ch);

            if !is_literal {
                if ch == open {
                    depth += 1;
                } else if ch == close {
                    depth -= 1;
                    if depth == 0 {
                        let content = self.input[content_start..content_end].to_string();
                        self.chars.next(); // consume closing delimiter
                        return Ok(content);
                    }
                }
            }
            content_end = pos + ch.len_utf8();
            self.chars.next();
        }

        Err(LexerError::UnterminatedSubstitution {
            span: Span::new(start, content_end),
        })
    }
}
