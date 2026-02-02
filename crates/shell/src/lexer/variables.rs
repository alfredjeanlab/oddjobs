// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Variable parsing ($VAR, ${VAR:-default}, etc.).

use super::{Lexer, LexerError};
use crate::token::{self, Span, Token, TokenKind};

/// Result of parsing a braced variable.
pub(super) struct BracedVariable {
    pub name: String,
    pub modifier: Option<String>,
}

impl Lexer<'_> {
    /// Lex a variable reference (`$VAR` or `${VAR}` or `${VAR:-default}`) or
    /// command substitution (`$(cmd)`).
    ///
    /// Called when peek() has confirmed the next char is '$'.
    pub(super) fn lex_variable(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next(); // consume $

        let Some(&(name_start, ch)) = self.chars.peek() else {
            return Err(LexerError::EmptyVariable {
                span: Span::new(start, start + 1),
            });
        };

        match ch {
            '(' => self.lex_dollar_substitution(start),
            '{' => self.lex_braced_variable(start),
            // Handle special single-character variables: $?, $$, $#, $0
            '?' | '$' | '#' | '0' => {
                self.chars.next(); // consume the special char
                Ok(Token::new(
                    TokenKind::Variable {
                        name: ch.to_string(),
                        modifier: None,
                    },
                    Span::new(start, name_start + 1),
                ))
            }
            _ => {
                // Check for empty variable ($ followed by non-name char)
                if !Self::is_valid_variable_start(ch) {
                    return Err(LexerError::EmptyVariable {
                        span: Span::new(start, start + 1),
                    });
                }
                self.lex_simple_variable(start, name_start)
            }
        }
    }

    /// Lex a simple variable reference (`$VAR`).
    fn lex_simple_variable(
        &mut self,
        start: usize,
        name_start: usize,
    ) -> Result<Token, LexerError> {
        let name = self.scan_variable_name(name_start);
        let end = name_start + name.len();
        Ok(Token::new(
            TokenKind::Variable {
                name,
                modifier: None,
            },
            Span::new(start, end),
        ))
    }

    /// Lex a braced variable reference (`${VAR}` or `${VAR:-default}`).
    fn lex_braced_variable(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next(); // consume {

        let var = self.parse_braced_variable(start)?;
        Ok(Token::new(
            TokenKind::Variable {
                name: var.name,
                modifier: var.modifier,
            },
            Span::new(start, self.current_position()),
        ))
    }

    /// Scan a variable name, consuming valid characters.
    ///
    /// Returns the variable name as a string.
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

    /// Parse braced variable internals (name and optional modifier).
    ///
    /// Called after `${` has been consumed. Handles the variable name and optional modifier,
    /// consuming through the closing `}`.
    pub(super) fn parse_braced_variable(
        &mut self,
        start: usize,
    ) -> Result<BracedVariable, LexerError> {
        let Some(&(name_start, first_ch)) = self.chars.peek() else {
            return Err(LexerError::UnterminatedVariable {
                span: Span::new(start, start + 2),
            });
        };

        // Check for empty ${} case
        if first_ch == '}' {
            self.chars.next();
            return Err(LexerError::EmptyVariable {
                span: Span::new(start, start + 3),
            });
        }

        // Check for special variable names: ${?}, ${$}, ${#}, ${0}
        if token::is_special_variable(first_ch) {
            self.chars.next(); // consume special char
            let name = first_ch.to_string();
            let name_end = name_start + first_ch.len_utf8();

            // Check for closing brace or modifier
            let Some(&(_, next_ch)) = self.chars.peek() else {
                return Err(LexerError::UnterminatedVariable {
                    span: Span::new(start, name_end),
                });
            };

            if next_ch == '}' {
                self.chars.next();
                return Ok(BracedVariable {
                    name,
                    modifier: None,
                });
            }

            // Has modifier - scan until closing brace with brace balancing
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
                            self.chars.next(); // consume }
                            return Ok(BracedVariable {
                                name,
                                modifier: Some(modifier),
                            });
                        }
                    }
                    _ => {}
                }
                mod_end = pos + ch.len_utf8();
                self.chars.next();
            }

            return Err(LexerError::UnterminatedVariable {
                span: Span::new(start, mod_end),
            });
        }

        // Check for invalid variable name start
        if !Self::is_valid_variable_start(first_ch) {
            return Err(LexerError::InvalidVariableName {
                name: first_ch.to_string(),
                span: Span::new(name_start, name_start + first_ch.len_utf8()),
            });
        }

        // Scan variable name
        let name = self.scan_variable_name(name_start);
        let name_end = name_start + name.len();

        // Check for modifier or closing brace
        let Some(&(_, next_ch)) = self.chars.peek() else {
            return Err(LexerError::UnterminatedVariable {
                span: Span::new(start, name_end),
            });
        };

        if next_ch == '}' {
            self.chars.next();
            return Ok(BracedVariable {
                name,
                modifier: None,
            });
        }

        // Has modifier - scan until closing brace with brace balancing
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
                        self.chars.next(); // consume }
                        return Ok(BracedVariable {
                            name,
                            modifier: Some(modifier),
                        });
                    }
                }
                _ => {}
            }
            mod_end = pos + ch.len_utf8();
            self.chars.next();
        }

        Err(LexerError::UnterminatedVariable {
            span: Span::new(start, mod_end),
        })
    }

    /// Check if a character is a valid start for a variable name.
    pub(super) fn is_valid_variable_start(ch: char) -> bool {
        token::is_valid_variable_start(ch)
    }

    /// Check if a character is valid within a variable name.
    pub(super) fn is_valid_variable_char(ch: char) -> bool {
        token::is_valid_variable_char(ch)
    }
}
