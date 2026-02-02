// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Word parsing: token→parts conversion, adjacent collection, assignments.

use super::Parser;
use crate::ast::{SubstitutionBody, Word, WordPart};
use crate::parse_error::ParseError;
use crate::token::{self, Span, Token, TokenKind};

impl Parser {
    /// Check if the next token is adjacent (no whitespace gap).
    #[inline]
    pub(super) fn is_adjacent(&self, current_end: usize) -> bool {
        self.peek()
            .map(|t| t.span.start == current_end)
            .unwrap_or(false)
    }

    /// Collect adjacent tokens into word parts.
    ///
    /// Extends `parts` with word parts from consecutive adjacent tokens,
    /// updating `end` to track the span end position.
    pub(super) fn collect_adjacent_parts(
        &mut self,
        end: &mut usize,
        parts: &mut Vec<WordPart>,
    ) -> Result<(), ParseError> {
        while self.is_adjacent(*end) {
            let token = match self.peek() {
                Some(t) => t.clone(),
                None => break,
            };
            let token_parts = self.token_to_parts(&token)?;
            if token_parts.is_empty() {
                break;
            }
            *end = token.span.end;
            parts.extend(token_parts);
            self.advance();
        }
        Ok(())
    }

    /// Parse a command substitution body and wrap it as a WordPart.
    fn parse_command_substitution(
        content: &str,
        backtick: bool,
        span: Span,
    ) -> Result<WordPart, ParseError> {
        let body = Parser::parse(content).map_err(|e| ParseError::InSubstitution {
            inner: Box::new(e),
            span,
        })?;
        Ok(WordPart::CommandSubstitution {
            body: SubstitutionBody::Parsed(Box::new(body)),
            backtick,
        })
    }

    /// Convert a token to WordParts.
    ///
    /// Returns an empty vec for non-word tokens, one or more parts for word tokens.
    pub(super) fn token_to_parts(&self, token: &Token) -> Result<Vec<WordPart>, ParseError> {
        match &token.kind {
            TokenKind::Word(s) => Ok(vec![WordPart::literal(s.clone())]),
            TokenKind::SingleQuoted(s) => Ok(vec![WordPart::single_quoted(s.clone())]),
            TokenKind::DoubleQuoted(word_parts) => {
                let mut parts = Vec::new();
                for wp in word_parts {
                    match wp {
                        WordPart::CommandSubstitution {
                            body: SubstitutionBody::Unparsed(content),
                            backtick,
                        } => {
                            parts.push(Self::parse_command_substitution(
                                content, *backtick, token.span,
                            )?);
                        }
                        other => parts.push(other.clone()),
                    }
                }
                Ok(parts)
            }
            TokenKind::Variable { name, modifier } => Ok(vec![WordPart::Variable {
                name: name.clone(),
                modifier: modifier.clone(),
            }]),
            TokenKind::CommandSubstitution { content, backtick } => {
                Ok(vec![Self::parse_command_substitution(
                    content, *backtick, token.span,
                )?])
            }
            _ => Ok(vec![]),
        }
    }

    /// Try to parse a word as an assignment (NAME=VALUE or NAME=).
    ///
    /// Returns `Some((name, value))` if the word is a valid assignment pattern,
    /// `None` otherwise. The name is the variable name, and value is everything
    /// after the `=` sign (may be empty).
    pub(super) fn try_parse_assignment_word(word: &str) -> Option<(&str, &str)> {
        let eq_pos = word.find('=')?;
        let name = &word[..eq_pos];
        let value = &word[eq_pos + 1..];

        // Validate variable name
        if !Self::is_valid_variable_name(name) {
            return None;
        }

        Some((name, value))
    }

    /// Check if a string is a valid shell variable name.
    ///
    /// Variable names start with `[a-zA-Z_]` and contain only `[a-zA-Z0-9_]`.
    pub(super) fn is_valid_variable_name(s: &str) -> bool {
        token::is_valid_variable_name(s)
    }

    /// Parse a word, concatenating adjacent tokens.
    pub(super) fn parse_word(&mut self) -> Result<Option<Word>, ParseError> {
        let first_token = match self.peek() {
            Some(t) => t.clone(),
            None => return Ok(None),
        };

        let first_parts = self.token_to_parts(&first_token)?;
        if first_parts.is_empty() {
            return Ok(None);
        }

        let start = first_token.span.start;
        let mut end = first_token.span.end;
        let mut parts = first_parts;
        self.advance();

        // Collect adjacent tokens into the same word
        self.collect_adjacent_parts(&mut end, &mut parts)?;

        Ok(Some(Word {
            parts,
            span: Span::new(start, end),
        }))
    }
}
