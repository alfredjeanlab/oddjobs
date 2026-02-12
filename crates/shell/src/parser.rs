// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shell parser that transforms tokens into an Abstract Syntax Tree.

use super::ast::*;
use super::lexer::Lexer;
use super::parse_error::{ParseError, ParseResult};
use super::token::{self, Span, Token, TokenKind};

enum CompoundDelimiter {
    Paren,
    Brace,
}

impl CompoundDelimiter {
    fn closing_token(&self) -> TokenKind {
        match self {
            CompoundDelimiter::Paren => TokenKind::RParen,
            CompoundDelimiter::Brace => TokenKind::RBrace,
        }
    }

    fn closing_str(&self) -> &'static str {
        match self {
            CompoundDelimiter::Paren => "')'",
            CompoundDelimiter::Brace => "'}'",
        }
    }
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    input_len: usize,
}

impl Parser {
    /// Parse input string into a command list, or error on invalid syntax.
    pub fn parse(input: &str) -> Result<CommandList, ParseError> {
        let tokens = Lexer::tokenize(input)?;
        let mut parser = Parser { tokens, pos: 0, input_len: input.len() };
        parser.parse_command_list()
    }

    /// Parse with error recovery: skips past errors to the next separator and
    /// resumes, returning all valid commands alongside collected errors.
    pub fn parse_with_recovery(input: &str) -> ParseResult {
        let tokens = match Lexer::tokenize(input) {
            Ok(t) => t,
            Err(e) => {
                return ParseResult {
                    commands: CommandList { commands: vec![], span: Span::empty(0) },
                    errors: vec![ParseError::Lexer(e)],
                };
            }
        };

        let mut parser = Parser { tokens, pos: 0, input_len: input.len() };
        parser.parse_with_recovery_inner()
    }

    fn parse_with_recovery_inner(&mut self) -> ParseResult {
        let start = self.current_span_start();
        let mut commands = Vec::new();
        let mut errors = Vec::new();

        self.skip_separators();

        while !self.at_end() {
            match self.parse_and_or_list() {
                Ok(and_or) => {
                    let last_was_background = and_or
                        .rest
                        .last()
                        .map(|(_, item)| item.background)
                        .unwrap_or(and_or.first.background);
                    commands.push(and_or);

                    if !last_was_background && !self.at_end() && !self.at_separator() {
                        errors.push(self.unexpected_token("';' or newline"));
                        self.recover_to_separator();
                    }
                }
                Err(e) => {
                    errors.push(e);
                    self.recover_to_separator();
                }
            }
            self.skip_separators();
        }

        let end = self.current_span_end();
        ParseResult {
            commands: CommandList { commands, span: Span::new(start, end.max(start)) },
            errors,
        }
    }

    /// Grammar: and_or_list ((';' | '&' | '\n') and_or_list)*
    fn parse_command_list(&mut self) -> Result<CommandList, ParseError> {
        self.parse_command_list_impl(false)
    }

    /// When `inner` is true, stops at group-ending tokens (`)`, `}`) without consuming them.
    fn parse_command_list_impl(&mut self, inner: bool) -> Result<CommandList, ParseError> {
        let start = self.current_span_start();
        let mut commands = Vec::new();

        self.skip_separators();

        while !(self.at_end() || (inner && self.at_group_end())) {
            let and_or = self.parse_and_or_list()?;

            let last_was_background = and_or
                .rest
                .last()
                .map(|(_, item)| item.background)
                .unwrap_or(and_or.first.background);

            commands.push(and_or);

            if !last_was_background && !self.at_end() && !self.at_separator() {
                if inner && self.at_group_end() {
                } else {
                    let expected =
                        if inner { "';', newline, or closing delimiter" } else { "';' or newline" };
                    return Err(self.unexpected_token(expected));
                }
            }
            self.skip_separators();
        }

        let end = self.current_span_end();
        Ok(CommandList { commands, span: Span::new(start, end.max(start)) })
    }

    /// Grammar: command_item (('&&' | '||') command_item)*
    ///
    /// AND/OR have equal precedence, left-associative.
    /// A backgrounded command (`cmd &`) terminates the list.
    fn parse_and_or_list(&mut self) -> Result<AndOrList, ParseError> {
        let first = self.parse_command_item()?;
        let start_span = first.span;
        let mut rest = Vec::new();

        if first.background {
            return Ok(AndOrList { first, rest, span: start_span });
        }

        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::And) => LogicalOp::And,
                Some(TokenKind::Or) => LogicalOp::Or,
                _ => break,
            };
            self.advance();
            let item = self.parse_command_item()?;
            let is_background = item.background;
            rest.push((op, item));

            if is_background {
                break;
            }
        }

        let end_span = rest.last().map(|(_, item)| item.span).unwrap_or(start_span);

        Ok(AndOrList { first, rest, span: start_span.merge(end_span) })
    }

    /// Grammar: job '&'?
    fn parse_command_item(&mut self) -> Result<CommandItem, ParseError> {
        let command = self.parse_job()?;
        let start_span = command.span();

        let (background, end_span) = match self.peek_kind() {
            Some(TokenKind::Ampersand) => {
                let span = self.tokens[self.pos].span;
                self.pos += 1;
                (true, span)
            }
            _ => (false, start_span),
        };

        Ok(CommandItem { command, background, span: start_span.merge(end_span) })
    }

    /// Grammar: compound_command ('|' simple_command)*
    ///
    /// Pipe binds tighter than && and ||.
    fn parse_job(&mut self) -> Result<Command, ParseError> {
        match self.peek_kind() {
            Some(TokenKind::LParen) => return self.parse_subshell(),
            Some(TokenKind::LBrace) => return self.parse_brace_group(),
            _ => {}
        }

        let first = self.parse_simple_command()?;

        if !matches!(self.peek_kind(), Some(TokenKind::Pipe)) {
            return Ok(Command::Simple(first));
        }

        let start_span = first.span;
        let mut end_span = first.span;
        let mut commands = vec![first];

        while matches!(self.peek_kind(), Some(TokenKind::Pipe)) {
            self.advance();
            let cmd = self.parse_simple_command()?;
            end_span = cmd.span;
            commands.push(cmd);
        }
        Ok(Command::Job(Job { commands, span: start_span.merge(end_span) }))
    }

    fn parse_subshell(&mut self) -> Result<Command, ParseError> {
        self.parse_compound_command(CompoundDelimiter::Paren)
    }

    /// POSIX requires a space after `{` and a `;` or newline before `}`.
    fn parse_brace_group(&mut self) -> Result<Command, ParseError> {
        self.parse_compound_command(CompoundDelimiter::Brace)
    }

    /// Opening delimiter must already be identified via peek_kind(); this consumes it.
    fn parse_compound_command(
        &mut self,
        delimiter: CompoundDelimiter,
    ) -> Result<Command, ParseError> {
        let start = self.tokens[self.pos].span.start;
        self.pos += 1;

        let body = self.parse_inner_command_list()?;

        match self.peek_kind() {
            Some(k) if *k == delimiter.closing_token() => {
                let mut end = self.tokens[self.pos].span.end;
                self.pos += 1;

                let mut redirections = Vec::new();
                while self.is_redirection_token() {
                    let redir = self.parse_redirection()?;
                    if let Some(span) = redir.target_span() {
                        end = span.end;
                    }
                    redirections.push(redir);
                }

                let span = Span::new(start, end);
                let boxed_body = Box::new(body);
                Ok(match delimiter {
                    CompoundDelimiter::Paren => {
                        Command::Subshell(Subshell { body: boxed_body, redirections, span })
                    }
                    CompoundDelimiter::Brace => {
                        Command::BraceGroup(BraceGroup { body: boxed_body, redirections, span })
                    }
                })
            }
            _ => Err(self.unexpected_token(delimiter.closing_str())),
        }
    }

    fn parse_inner_command_list(&mut self) -> Result<CommandList, ParseError> {
        self.parse_command_list_impl(true)
    }

    /// Grammar: assignment* word word*
    ///
    /// Word tokens matching NAME=VALUE at command-start position are parsed as assignments.
    fn parse_simple_command(&mut self) -> Result<SimpleCommand, ParseError> {
        let start_span = self.peek().map(|t| t.span).unwrap_or_else(|| Span::empty(0));

        let mut env = Vec::new();
        while let Some(Token { kind: TokenKind::Word(word), span }) = self.peek().cloned() {
            let Some((name, value_after_eq)) = Self::try_parse_assignment_word(&word) else {
                break;
            };

            self.advance();

            let value_start = span.start + name.len() + 1; // After "NAME="
            let mut value_end = span.end;
            let mut parts = Vec::new();

            if !value_after_eq.is_empty() {
                parts.push(WordPart::literal(value_after_eq.to_string()));
            }

            self.collect_adjacent_parts(&mut value_end, &mut parts)?;

            // Empty literal for the bare `VAR=` case
            if parts.is_empty() {
                parts.push(WordPart::literal(String::new()));
            }

            env.push(EnvAssignment {
                name: name.to_string(),
                value: Word { parts, span: Span::new(value_start, value_end) },
                span,
            });
        }

        match self.parse_word()? {
            Some(name) => {
                let mut args = Vec::new();
                let mut redirections = Vec::new();
                let mut end_span = name.span;

                loop {
                    if self.is_redirection_token() {
                        let redir = self.parse_redirection()?;
                        if let Some(span) = redir.target_span() {
                            end_span = span;
                        }
                        redirections.push(redir);
                    } else if let Some(word) = self.parse_word()? {
                        end_span = word.span;
                        args.push(word);
                    } else {
                        break;
                    }
                }

                let span = start_span.merge(end_span);
                Ok(SimpleCommand { env, name, args, redirections, span })
            }
            None => {
                if !env.is_empty() {
                    Ok(SimpleCommand {
                        env,
                        name: Word { parts: vec![], span: Span::empty(start_span.start) },
                        args: vec![],
                        redirections: vec![],
                        span: start_span,
                    })
                } else {
                    Err(self.unexpected_token("command"))
                }
            }
        }
    }

    #[inline]
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    #[inline]
    fn peek_kind(&self) -> Option<&TokenKind> {
        self.peek().map(|t| &t.kind)
    }

    #[inline]
    fn advance(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.pos);
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    #[inline]
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    #[inline]
    fn at_separator(&self) -> bool {
        matches!(self.peek_kind(), Some(TokenKind::Semi | TokenKind::Newline))
    }

    #[inline]
    fn at_group_end(&self) -> bool {
        matches!(self.peek_kind(), Some(TokenKind::RParen | TokenKind::RBrace))
    }

    fn skip_separators(&mut self) {
        while self.at_separator() {
            self.advance();
        }
    }

    fn current_span_start(&self) -> usize {
        self.peek().map(|t| t.span.start).unwrap_or(0)
    }

    fn current_span_end(&self) -> usize {
        if self.pos > 0 {
            self.tokens[self.pos - 1].span.end
        } else if !self.tokens.is_empty() {
            0
        } else {
            self.input_len
        }
    }

    fn unexpected_token(&self, expected: &str) -> ParseError {
        match self.peek() {
            Some(token) => ParseError::UnexpectedToken {
                found: token.kind.clone(),
                expected: expected.to_string(),
                span: token.span,
            },
            None => ParseError::UnexpectedEof { expected: expected.to_string() },
        }
    }

    fn recover_to_separator(&mut self) {
        // Skip leading operators that likely caused the error, then advance to next separator
        while !self.at_end() {
            match self.peek_kind() {
                Some(TokenKind::Pipe | TokenKind::And | TokenKind::Or | TokenKind::Ampersand) => {
                    self.advance();
                }
                _ => break,
            }
        }

        while !self.at_end() && !self.at_separator() {
            self.advance();
        }
    }
}

// ── Redirections ─────────────────────────────────────────────────────────

impl Parser {
    fn is_redirection_token(&self) -> bool {
        self.peek_kind().is_some_and(TokenKind::is_redirection)
    }

    fn parse_redirection(&mut self) -> Result<Redirection, ParseError> {
        let token = match self.peek() {
            Some(t) => t.clone(),
            None => unreachable!("is_redirection_token verified token exists"),
        };
        self.advance();

        Ok(match token.kind {
            TokenKind::RedirectOut { fd } => {
                let target =
                    self.parse_word()?.ok_or_else(|| self.unexpected_token("redirect target"))?;
                Redirection::Out { fd, target, append: false }
            }
            TokenKind::RedirectAppend { fd } => {
                let target =
                    self.parse_word()?.ok_or_else(|| self.unexpected_token("redirect target"))?;
                Redirection::Out { fd, target, append: true }
            }
            TokenKind::RedirectIn { fd } => {
                let source =
                    self.parse_word()?.ok_or_else(|| self.unexpected_token("redirect source"))?;
                Redirection::In { fd, source }
            }
            TokenKind::HereDoc { fd, strip_tabs, delimiter, body, quoted } => {
                Redirection::HereDoc { fd, delimiter, body, strip_tabs, quoted }
            }
            TokenKind::HereString { fd } => {
                let content = self
                    .parse_word()?
                    .ok_or_else(|| self.unexpected_token("here-string content"))?;
                Redirection::HereString { fd, content }
            }
            TokenKind::RedirectBoth { append } => {
                let target =
                    self.parse_word()?.ok_or_else(|| self.unexpected_token("redirect target"))?;
                Redirection::Both { append, target }
            }
            TokenKind::DuplicateFd { source, target, output } => {
                Redirection::Duplicate { source, target, output }
            }
            _ => unreachable!("is_redirection_token already verified"),
        })
    }
}

// ── Words ────────────────────────────────────────────────────────────────

impl Parser {
    #[inline]
    fn is_adjacent(&self, current_end: usize) -> bool {
        self.peek().map(|t| t.span.start == current_end).unwrap_or(false)
    }

    fn collect_adjacent_parts(
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

    fn parse_command_substitution(
        content: &str,
        backtick: bool,
        span: Span,
    ) -> Result<WordPart, ParseError> {
        let body = Parser::parse(content)
            .map_err(|e| ParseError::InSubstitution { inner: Box::new(e), span })?;
        Ok(WordPart::CommandSubstitution {
            body: SubstitutionBody::Parsed(Box::new(body)),
            backtick,
        })
    }

    /// Returns an empty vec for non-word tokens, one or more parts for word tokens.
    fn token_to_parts(&self, token: &Token) -> Result<Vec<WordPart>, ParseError> {
        match &token.kind {
            TokenKind::Word(s) => Ok(vec![WordPart::literal(s.clone())]),
            TokenKind::SingleQuoted(s) => Ok(vec![WordPart::single_quoted(s.clone())]),
            TokenKind::DoubleQuoted(word_parts) => {
                if word_parts.is_empty() {
                    return Ok(vec![WordPart::double_quoted("")]);
                }
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
            TokenKind::Variable { name, modifier } => {
                Ok(vec![WordPart::Variable { name: name.clone(), modifier: modifier.clone() }])
            }
            TokenKind::CommandSubstitution { content, backtick } => {
                Ok(vec![Self::parse_command_substitution(content, *backtick, token.span)?])
            }
            _ => Ok(vec![]),
        }
    }

    fn try_parse_assignment_word(word: &str) -> Option<(&str, &str)> {
        let eq_pos = word.find('=')?;
        let name = &word[..eq_pos];
        let value = &word[eq_pos + 1..];

        if !Self::is_valid_variable_name(name) {
            return None;
        }

        Some((name, value))
    }

    fn is_valid_variable_name(s: &str) -> bool {
        token::is_valid_variable_name(s)
    }

    fn parse_word(&mut self) -> Result<Option<Word>, ParseError> {
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

        self.collect_adjacent_parts(&mut end, &mut parts)?;

        Ok(Some(Word { parts, span: Span::new(start, end) }))
    }
}

#[cfg(test)]
#[path = "parser_tests/mod.rs"]
mod tests;
