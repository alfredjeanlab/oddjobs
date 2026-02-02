// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Redirection detection and parsing.

use super::Parser;
use crate::ast::Redirection;
use crate::parse_error::ParseError;
use crate::token::TokenKind;

impl Parser {
    /// Check if the next token is a redirection token.
    pub(super) fn is_redirection_token(&self) -> bool {
        self.peek_kind().is_some_and(TokenKind::is_redirection)
    }

    /// Parse a redirection.
    pub(super) fn parse_redirection(&mut self) -> Result<Redirection, ParseError> {
        // is_redirection_token() already verified token exists
        let token = match self.peek() {
            Some(t) => t.clone(),
            None => unreachable!("is_redirection_token verified token exists"),
        };
        self.advance();

        Ok(match token.kind {
            TokenKind::RedirectOut { fd } => {
                let target = self
                    .parse_word()?
                    .ok_or_else(|| self.unexpected_token("redirect target"))?;
                Redirection::Out {
                    fd,
                    target,
                    append: false,
                }
            }
            TokenKind::RedirectAppend { fd } => {
                let target = self
                    .parse_word()?
                    .ok_or_else(|| self.unexpected_token("redirect target"))?;
                Redirection::Out {
                    fd,
                    target,
                    append: true,
                }
            }
            TokenKind::RedirectIn { fd } => {
                let source = self
                    .parse_word()?
                    .ok_or_else(|| self.unexpected_token("redirect source"))?;
                Redirection::In { fd, source }
            }
            TokenKind::HereDoc {
                fd,
                strip_tabs,
                delimiter,
                body,
                quoted,
            } => Redirection::HereDoc {
                fd,
                delimiter,
                body,
                strip_tabs,
                quoted,
            },
            TokenKind::HereString { fd } => {
                let content = self
                    .parse_word()?
                    .ok_or_else(|| self.unexpected_token("here-string content"))?;
                Redirection::HereString { fd, content }
            }
            TokenKind::RedirectBoth { append } => {
                let target = self
                    .parse_word()?
                    .ok_or_else(|| self.unexpected_token("redirect target"))?;
                Redirection::Both { append, target }
            }
            TokenKind::DuplicateFd {
                source,
                target,
                output,
            } => Redirection::Duplicate {
                source,
                target,
                output,
            },
            _ => unreachable!("is_redirection_token already verified"),
        })
    }
}
