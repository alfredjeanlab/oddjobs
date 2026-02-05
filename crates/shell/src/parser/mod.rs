// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shell parser that transforms tokens into an Abstract Syntax Tree.

mod redirections;
mod words;

use super::ast::*;
use super::lexer::Lexer;
use super::parse_error::{ParseError, ParseResult};
use super::token::{Span, Token, TokenKind};

/// Delimiter kind for compound commands.
enum CompoundDelimiter {
    /// ( ... ) subshell
    Paren,
    /// { ... } brace group
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

/// Shell parser that transforms tokens into an AST.
///
/// The parser performs two-step processing:
/// 1. Lexing: The input is tokenized into words, operators, and control characters
/// 2. Parsing: Tokens are assembled into an AST following shell grammar rules
///
/// # Examples
///
/// ## Basic parsing
///
/// ```ignore
/// use oj_shell::Parser;
///
/// let ast = Parser::parse("echo hello world")?;
/// assert_eq!(ast.count_simple_commands(), 1);
/// # Ok::<(), oj_shell::ParseError>(())
/// ```
///
/// ## Error recovery
///
/// ```ignore
/// use oj_shell::Parser;
///
/// let result = Parser::parse_with_recovery("echo hello; | bad; echo ok");
/// assert_eq!(result.commands.count_simple_commands(), 2); // echo hello + echo ok
/// assert_eq!(result.errors.len(), 1); // | bad
/// ```
pub struct Parser {
    /// The tokens to parse.
    tokens: Vec<Token>,
    /// Current position in the token stream.
    pos: usize,
    /// Length of the original input (for span calculation).
    input_len: usize,
}

impl Parser {
    /// Parse input string into a command list.
    ///
    /// Returns an error if the input is syntactically invalid. For lenient
    /// parsing that continues past errors, use [`Parser::parse_with_recovery`].
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use oj_shell::Parser;
    ///
    /// // Simple command
    /// let ast = Parser::parse("echo hello")?;
    /// assert_eq!(ast.count_simple_commands(), 1);
    ///
    /// // Job
    /// let ast = Parser::parse("cat file | grep pattern | wc -l")?;
    /// assert_eq!(ast.count_simple_commands(), 3);
    ///
    /// // AND/OR chain
    /// let ast = Parser::parse("test -f file && cat file || echo missing")?;
    /// assert_eq!(ast.count_simple_commands(), 3);
    ///
    /// // Variables and substitutions
    /// let ast = Parser::parse("echo $HOME $(date)")?;
    /// assert_eq!(ast.collect_variables(), vec!["HOME"]);
    /// assert!(ast.has_command_substitutions());
    /// # Ok::<(), oj_shell::ParseError>(())
    /// ```
    pub fn parse(input: &str) -> Result<CommandList, ParseError> {
        let tokens = Lexer::tokenize(input)?;
        let mut parser = Parser {
            tokens,
            pos: 0,
            input_len: input.len(),
        };
        parser.parse_command_list()
    }

    /// Parse with error recovery, returning all valid commands and collected errors.
    ///
    /// Unlike [`Parser::parse`], this continues after errors by skipping to the
    /// next separator and resuming. Useful for IDE integration or diagnostics.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use oj_shell::Parser;
    ///
    /// // Error in the middle - valid commands still returned
    /// let result = Parser::parse_with_recovery("echo hello; | bad; echo world");
    /// assert_eq!(result.commands.count_simple_commands(), 2);
    /// assert_eq!(result.errors.len(), 1);
    ///
    /// // Multiple errors collected
    /// let result = Parser::parse_with_recovery("echo ok; && bad; || bad; echo done");
    /// assert_eq!(result.commands.count_simple_commands(), 2);
    /// assert_eq!(result.errors.len(), 2);
    /// ```
    pub fn parse_with_recovery(input: &str) -> ParseResult {
        let tokens = match Lexer::tokenize(input) {
            Ok(t) => t,
            Err(e) => {
                return ParseResult {
                    commands: CommandList {
                        commands: vec![],
                        span: Span::empty(0),
                    },
                    errors: vec![ParseError::Lexer(e)],
                };
            }
        };

        let mut parser = Parser {
            tokens,
            pos: 0,
            input_len: input.len(),
        };
        parser.parse_with_recovery_inner()
    }

    /// Internal parse with recovery.
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

                    // If not backgrounded and not at separator, try to recover
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
            commands: CommandList {
                commands,
                span: Span::new(start, end.max(start)),
            },
            errors,
        }
    }

    /// Parse a command list (top-level entry point).
    ///
    /// Grammar: and_or_list ((';' | '&' | '\n') and_or_list)*
    /// The `&` acts as both a background operator and a command separator.
    fn parse_command_list(&mut self) -> Result<CommandList, ParseError> {
        self.parse_command_list_impl(false)
    }

    /// Parse a command list.
    ///
    /// When `inner` is true, stops at group-ending tokens (`)`, `}`) without consuming them.
    /// When `inner` is false, parses until end of input.
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
                    // At closing delimiter - valid stopping point
                } else {
                    let expected = if inner {
                        "';', newline, or closing delimiter"
                    } else {
                        "';' or newline"
                    };
                    return Err(self.unexpected_token(expected));
                }
            }
            self.skip_separators();
        }

        let end = self.current_span_end();
        Ok(CommandList {
            commands,
            span: Span::new(start, end.max(start)),
        })
    }

    /// Parse an and-or list: command_item (('&&' | '||') command_item)*
    ///
    /// AND and OR have equal precedence and are left-associative.
    /// A backgrounded command (`cmd &`) terminates the and_or_list - no `&&`/`||` after it.
    fn parse_and_or_list(&mut self) -> Result<AndOrList, ParseError> {
        let first = self.parse_command_item()?;
        let start_span = first.span;
        let mut rest = Vec::new();

        // If first command is backgrounded, the and_or_list ends here
        // (e.g., `a & && b` -> `a &` is complete, `&& b` is an error for the next parse)
        if first.background {
            return Ok(AndOrList {
                first,
                rest,
                span: start_span,
            });
        }

        loop {
            let op = match self.peek_kind() {
                Some(TokenKind::And) => LogicalOp::And,
                Some(TokenKind::Or) => LogicalOp::Or,
                _ => break,
            };
            self.advance(); // consume && or ||
            let item = self.parse_command_item()?;
            let is_background = item.background;
            rest.push((op, item));

            // If this item is backgrounded, stop the chain
            if is_background {
                break;
            }
        }

        let end_span = rest.last().map(|(_, item)| item.span).unwrap_or(start_span);

        Ok(AndOrList {
            first,
            rest,
            span: start_span.merge(end_span),
        })
    }

    /// Parse a command with optional background: job '&'?
    ///
    /// Background applies only to the immediately preceding job.
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

        Ok(CommandItem {
            command,
            background,
            span: start_span.merge(end_span),
        })
    }

    /// Parse a job: compound_command ('|' simple_command)*
    ///
    /// Pipe binds tighter than && and ||.
    /// Note: First element can be a compound command, but job elements must be simple.
    fn parse_job(&mut self) -> Result<Command, ParseError> {
        // Check for compound commands first (subshell, brace group)
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
            self.advance(); // consume |
            let cmd = self.parse_simple_command()?;
            end_span = cmd.span;
            commands.push(cmd);
        }
        Ok(Command::Job(Job {
            commands,
            span: start_span.merge(end_span),
        }))
    }

    /// Parse a subshell: `(` command_list `)`.
    fn parse_subshell(&mut self) -> Result<Command, ParseError> {
        self.parse_compound_command(CompoundDelimiter::Paren)
    }

    /// Parse a brace group: `{` command_list `}`.
    ///
    /// Note: POSIX requires a space after `{` and a `;` or newline before `}`.
    fn parse_brace_group(&mut self) -> Result<Command, ParseError> {
        self.parse_compound_command(CompoundDelimiter::Brace)
    }

    /// Parse a compound command (subshell or brace group).
    /// Opening delimiter must already be identified; this consumes it.
    fn parse_compound_command(
        &mut self,
        delimiter: CompoundDelimiter,
    ) -> Result<Command, ParseError> {
        // Caller verified via peek_kind(), so token exists at current position
        let start = self.tokens[self.pos].span.start;
        self.pos += 1;

        let body = self.parse_inner_command_list()?;

        match self.peek_kind() {
            Some(k) if *k == delimiter.closing_token() => {
                // peek_kind() confirmed token exists
                let mut end = self.tokens[self.pos].span.end;
                self.pos += 1;

                // Parse redirections after the compound command
                let mut redirections = Vec::new();
                while self.is_redirection_token() {
                    let redir = self.parse_redirection()?;
                    // Update end span based on redirect target/source
                    if let Some(span) = redir.target_span() {
                        end = span.end;
                    }
                    redirections.push(redir);
                }

                let span = Span::new(start, end);
                let boxed_body = Box::new(body);
                Ok(match delimiter {
                    CompoundDelimiter::Paren => Command::Subshell(Subshell {
                        body: boxed_body,
                        redirections,
                        span,
                    }),
                    CompoundDelimiter::Brace => Command::BraceGroup(BraceGroup {
                        body: boxed_body,
                        redirections,
                        span,
                    }),
                })
            }
            _ => Err(self.unexpected_token(delimiter.closing_str())),
        }
    }

    /// Parse commands until a closing delimiter is reached.
    /// Does NOT consume the closing delimiter.
    fn parse_inner_command_list(&mut self) -> Result<CommandList, ParseError> {
        self.parse_command_list_impl(true)
    }

    /// Parse a simple command: assignment* word word*
    ///
    /// Collects environment variable assignments before the command name.
    /// Assignment detection is done at parser level: Word tokens matching
    /// NAME=VALUE pattern at command-start position are parsed as assignments.
    fn parse_simple_command(&mut self) -> Result<SimpleCommand, ParseError> {
        let start_span = self
            .peek()
            .map(|t| t.span)
            .unwrap_or_else(|| Span::empty(0));

        // Collect environment assignments from Word tokens at command-start
        let mut env = Vec::new();
        while let Some(Token {
            kind: TokenKind::Word(word),
            span,
        }) = self.peek().cloned()
        {
            // Try to parse as assignment: NAME=VALUE or NAME=
            let Some((name, value_after_eq)) = Self::try_parse_assignment_word(&word) else {
                break; // Not an assignment, done collecting
            };

            self.advance();

            // Start building the value word parts
            let value_start = span.start + name.len() + 1; // After "NAME="
            let mut value_end = span.end;
            let mut parts = Vec::new();

            // Add the literal part from the word (if non-empty)
            if !value_after_eq.is_empty() {
                parts.push(WordPart::literal(value_after_eq.to_string()));
            }

            // Collect adjacent tokens (like parse_word does)
            self.collect_adjacent_parts(&mut value_end, &mut parts)?;

            // If no parts were collected, add an empty literal (for VAR= case)
            if parts.is_empty() {
                parts.push(WordPart::literal(String::new()));
            }

            env.push(EnvAssignment {
                name: name.to_string(),
                value: Word {
                    parts,
                    span: Span::new(value_start, value_end),
                },
                span,
            });
        }

        // Parse command name
        match self.parse_word()? {
            Some(name) => {
                let mut args = Vec::new();
                let mut redirections = Vec::new();
                let mut end_span = name.span;

                // Parse arguments and redirections
                loop {
                    if self.is_redirection_token() {
                        let redir = self.parse_redirection()?;
                        // Update end_span based on redirect target/source
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
                Ok(SimpleCommand {
                    env,
                    name,
                    args,
                    redirections,
                    span,
                })
            }
            None => {
                // Token present but not a word
                if !env.is_empty() {
                    // Standalone assignment (bash allows this)
                    Ok(SimpleCommand {
                        env,
                        name: Word {
                            parts: vec![],
                            span: Span::empty(start_span.start),
                        },
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

    /// Peek at the current token without consuming it.
    #[inline]
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// Peek at the kind of the current token.
    #[inline]
    fn peek_kind(&self) -> Option<&TokenKind> {
        self.peek().map(|t| &t.kind)
    }

    /// Advance to the next token.
    #[inline]
    fn advance(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.pos);
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    /// Check if we're at the end of input.
    #[inline]
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    /// Check if current token is a separator.
    #[inline]
    fn at_separator(&self) -> bool {
        matches!(self.peek_kind(), Some(TokenKind::Semi | TokenKind::Newline))
    }

    /// Check if at a group-ending token.
    #[inline]
    fn at_group_end(&self) -> bool {
        matches!(
            self.peek_kind(),
            Some(TokenKind::RParen | TokenKind::RBrace)
        )
    }

    /// Skip separator tokens.
    fn skip_separators(&mut self) {
        while self.at_separator() {
            self.advance();
        }
    }

    /// Get the start position for the current span.
    fn current_span_start(&self) -> usize {
        self.peek().map(|t| t.span.start).unwrap_or(0)
    }

    /// Get the end position for the current span.
    fn current_span_end(&self) -> usize {
        if self.pos > 0 {
            self.tokens[self.pos - 1].span.end
        } else if !self.tokens.is_empty() {
            // No tokens consumed yet, use input length if we have tokens
            0
        } else {
            self.input_len
        }
    }

    /// Create an unexpected token error.
    fn unexpected_token(&self, expected: &str) -> ParseError {
        match self.peek() {
            Some(token) => ParseError::UnexpectedToken {
                found: token.kind.clone(),
                expected: expected.to_string(),
                span: token.span,
            },
            None => ParseError::UnexpectedEof {
                expected: expected.to_string(),
            },
        }
    }

    /// Skip tokens until we find a separator (error recovery).
    ///
    /// Skips to the next `;` or newline, consuming problematic tokens along the way.
    fn recover_to_separator(&mut self) {
        // First, skip any leading operators that might have caused the error
        while !self.at_end() {
            match self.peek_kind() {
                Some(TokenKind::Pipe | TokenKind::And | TokenKind::Or | TokenKind::Ampersand) => {
                    self.advance();
                }
                _ => break,
            }
        }

        // Then skip to the next separator
        while !self.at_end() && !self.at_separator() {
            self.advance();
        }
    }
}

#[cfg(test)]
#[path = "../parser_tests/mod.rs"]
mod tests;
