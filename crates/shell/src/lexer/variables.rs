// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Variable parsing ($VAR, ${VAR:-default}, etc.).

use super::{Lexer, LexerError};
use crate::token::{self, Span, Token, TokenKind};

pub(super) struct BracedVariable {
    pub name: String,
    pub modifier: Option<String>,
}

impl Lexer<'_> {
    pub(super) fn lex_variable(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next();

        let Some(&(name_start, ch)) = self.chars.peek() else {
            return Err(LexerError::EmptyVariable { span: Span::new(start, start + 1) });
        };

        match ch {
            '(' => self.lex_dollar_substitution(start),
            '{' => self.lex_braced_variable(start),
            '?' | '$' | '#' | '0' => {
                self.chars.next();
                Ok(Token::new(
                    TokenKind::Variable { name: ch.to_string(), modifier: None },
                    Span::new(start, name_start + 1),
                ))
            }
            _ => {
                if !Self::is_valid_variable_start(ch) {
                    return Err(LexerError::EmptyVariable { span: Span::new(start, start + 1) });
                }
                self.lex_simple_variable(start, name_start)
            }
        }
    }

    fn lex_simple_variable(
        &mut self,
        start: usize,
        name_start: usize,
    ) -> Result<Token, LexerError> {
        let name = self.scan_variable_name(name_start);
        let end = name_start + name.len();
        Ok(Token::new(TokenKind::Variable { name, modifier: None }, Span::new(start, end)))
    }

    fn lex_braced_variable(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next();

        let var = self.parse_braced_variable(start)?;
        Ok(Token::new(
            TokenKind::Variable { name: var.name, modifier: var.modifier },
            Span::new(start, self.current_position()),
        ))
    }

    pub(super) fn scan_variable_name(&mut self, start: usize) -> String {
        let mut end = start;

        while let Some(&(pos, ch)) = self.chars.peek() {
            if !Self::is_valid_variable_char(ch) {
                break;
            }
            end = pos + ch.len_utf8();
            self.chars.next();
        }

        self.input[start..end].to_string()
    }

    /// Parse braced variable internals (name and optional modifier), consuming through `}`.
    pub(super) fn parse_braced_variable(
        &mut self,
        start: usize,
    ) -> Result<BracedVariable, LexerError> {
        let Some(&(name_start, first_ch)) = self.chars.peek() else {
            return Err(LexerError::UnterminatedVariable { span: Span::new(start, start + 2) });
        };

        if first_ch == '}' {
            self.chars.next();
            return Err(LexerError::EmptyVariable { span: Span::new(start, start + 3) });
        }

        if token::is_special_variable(first_ch) {
            self.chars.next();
            let name = first_ch.to_string();
            let name_end = name_start + first_ch.len_utf8();

            let Some(&(_, next_ch)) = self.chars.peek() else {
                return Err(LexerError::UnterminatedVariable { span: Span::new(start, name_end) });
            };

            if next_ch == '}' {
                self.chars.next();
                return Ok(BracedVariable { name, modifier: None });
            }

            let mod_start = name_end;
            let mut mod_end = mod_start;
            let mut depth = 1;

            while let Some(&(pos, ch)) = self.chars.peek() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            let modifier = self.input[mod_start..mod_end].to_string();
                            self.chars.next();
                            return Ok(BracedVariable { name, modifier: Some(modifier) });
                        }
                    }
                    _ => {}
                }
                mod_end = pos + ch.len_utf8();
                self.chars.next();
            }

            return Err(LexerError::UnterminatedVariable { span: Span::new(start, mod_end) });
        }

        if !Self::is_valid_variable_start(first_ch) {
            return Err(LexerError::InvalidVariableName {
                name: first_ch.to_string(),
                span: Span::new(name_start, name_start + first_ch.len_utf8()),
            });
        }

        let name = self.scan_variable_name(name_start);
        let name_end = name_start + name.len();

        let Some(&(_, next_ch)) = self.chars.peek() else {
            return Err(LexerError::UnterminatedVariable { span: Span::new(start, name_end) });
        };

        if next_ch == '}' {
            self.chars.next();
            return Ok(BracedVariable { name, modifier: None });
        }

        let mod_start = name_end;
        let mut mod_end = mod_start;
        let mut depth = 1;

        while let Some(&(pos, ch)) = self.chars.peek() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let modifier = self.input[mod_start..mod_end].to_string();
                        self.chars.next();
                        return Ok(BracedVariable { name, modifier: Some(modifier) });
                    }
                }
                _ => {}
            }
            mod_end = pos + ch.len_utf8();
            self.chars.next();
        }

        Err(LexerError::UnterminatedVariable { span: Span::new(start, mod_end) })
    }

    pub(super) fn is_valid_variable_start(ch: char) -> bool {
        token::is_valid_variable_start(ch)
    }

    pub(super) fn is_valid_variable_char(ch: char) -> bool {
        token::is_valid_variable_char(ch)
    }
}
