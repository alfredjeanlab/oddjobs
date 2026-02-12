// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Command substitution ($(...) and backticks).

use super::{Lexer, LexerError, QuoteState};
use crate::token::{Span, Token, TokenKind};

impl Lexer<'_> {
    /// Tracks ALL parentheses for balance, not just `$()` markers, because shell
    /// command substitution must balance all parentheses. Content is stored as a raw
    /// string (lazy parsing) rather than recursively parsed.
    pub(super) fn lex_dollar_substitution(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next();

        let content = self.read_balanced_content('(', ')', start)?;
        Ok(Token::new(
            TokenKind::CommandSubstitution { content, backtick: false },
            Span::new(start, self.current_position()),
        ))
    }

    pub(super) fn lex_backtick_substitution(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next();

        let content = self.read_backtick_content(start)?;
        Ok(Token::new(
            TokenKind::CommandSubstitution { content, backtick: true },
            Span::new(start, self.current_position()),
        ))
    }

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
                    self.chars.next();
                    return Ok(content);
                }
                _ => {
                    content_end = pos + ch.len_utf8();
                    self.chars.next();
                }
            }
        }

        Err(LexerError::UnterminatedSubstitution { span: Span::new(start, content_end) })
    }

    /// Read balanced content until closing delimiter, tracking nested parens/braces
    /// and respecting quotes.
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
                        self.chars.next();
                        return Ok(content);
                    }
                }
            }
            content_end = pos + ch.len_utf8();
            self.chars.next();
        }

        Err(LexerError::UnterminatedSubstitution { span: Span::new(start, content_end) })
    }
}
