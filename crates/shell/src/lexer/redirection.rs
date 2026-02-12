// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Redirection and heredoc lexing.

use super::{Lexer, LexerError};
use crate::token::{DupTarget, Span, Token, TokenKind};

impl Lexer<'_> {
    pub(super) fn capture_pending_heredocs(
        &mut self,
        tokens: &mut [Token],
    ) -> Result<(), LexerError> {
        while let Some(pending) = self.pending_heredocs.pop_front() {
            let body = match self.read_heredoc_body(&pending.delimiter, pending.strip_tabs)? {
                Some(body) => body,
                None => {
                    return Err(LexerError::UnterminatedHereDoc {
                        delimiter: pending.delimiter,
                        span: pending.span,
                    });
                }
            };

            if let TokenKind::HereDoc { body: ref mut token_body, .. } =
                tokens[pending.token_index].kind
            {
                *token_body = body;
            }
        }
        Ok(())
    }

    /// Returns `Ok(Some(body))` if delimiter was found, `Ok(None)` if EOF without delimiter.
    fn read_heredoc_body(
        &mut self,
        delimiter: &str,
        strip_tabs: bool,
    ) -> Result<Option<String>, LexerError> {
        let mut body = String::new();

        loop {
            if self.chars.peek().is_none() {
                return Ok(None);
            }

            let line = self.read_line();

            let check = if strip_tabs { line.trim_start_matches('\t') } else { &line };

            let check_trimmed = check.strip_suffix('\n').unwrap_or(check);
            let check_trimmed = check_trimmed.strip_suffix('\r').unwrap_or(check_trimmed);

            if check_trimmed == delimiter {
                return Ok(Some(body));
            }

            let content = if strip_tabs { line.trim_start_matches('\t') } else { &line };
            body.push_str(content);
        }
    }

    pub(super) fn read_line(&mut self) -> String {
        let start = self.current_position();
        let mut end = start;

        while let Some(&(pos, ch)) = self.chars.peek() {
            if ch == '\n' || ch == '\r' {
                end = pos + self.consume_newline();
                break;
            }
            end = pos + ch.len_utf8();
            self.chars.next();
        }

        self.input[start..end].to_string()
    }

    fn fd_prefix_len(fd: Option<u32>) -> usize {
        fd.map(|f| f.to_string().len()).unwrap_or(0)
    }

    /// Lex output redirection: `>`, `>>`, or `>&`.
    pub(super) fn lex_redirect_out(
        &mut self,
        start: usize,
        fd: Option<u32>,
    ) -> Result<Token, LexerError> {
        self.chars.next();
        let fd_len = Self::fd_prefix_len(fd);

        match self.peek_char() {
            Some('>') => {
                self.chars.next();
                Ok(Token::new(
                    TokenKind::RedirectAppend { fd },
                    Span::new(start, start + fd_len + 2),
                ))
            }
            Some('&') => {
                self.chars.next();
                self.lex_dup_target(start, fd.unwrap_or(1), true, fd_len + 2)
            }
            _ => {
                Ok(Token::new(TokenKind::RedirectOut { fd }, Span::new(start, start + fd_len + 1)))
            }
        }
    }

    /// Lex input redirection: `<`, `<<`, `<<-`, `<<<`, or `<&`.
    pub(super) fn lex_redirect_in(
        &mut self,
        start: usize,
        fd: Option<u32>,
    ) -> Result<Token, LexerError> {
        self.chars.next();
        let fd_len = Self::fd_prefix_len(fd);

        match self.peek_char() {
            Some('<') => {
                self.chars.next();
                match self.peek_char() {
                    Some('<') => {
                        self.chars.next();
                        Ok(Token::new(
                            TokenKind::HereString { fd },
                            Span::new(start, start + fd_len + 3),
                        ))
                    }
                    Some('-') => {
                        self.chars.next();
                        self.lex_heredoc(start, fd, true, fd_len + 3)
                    }
                    _ => self.lex_heredoc(start, fd, false, fd_len + 2),
                }
            }
            Some('&') => {
                self.chars.next();
                self.lex_dup_target(start, fd.unwrap_or(0), false, fd_len + 2)
            }
            _ => Ok(Token::new(TokenKind::RedirectIn { fd }, Span::new(start, start + fd_len + 1))),
        }
    }

    fn lex_heredoc(
        &mut self,
        start: usize,
        fd: Option<u32>,
        strip_tabs: bool,
        prefix_len: usize,
    ) -> Result<Token, LexerError> {
        self.skip_whitespace();

        let (delimiter, quoted) = self.read_heredoc_delimiter()?;

        if delimiter.is_empty() {
            return Err(LexerError::InvalidRedirection {
                message: "expected delimiter after <<".to_string(),
                span: Span::new(start, start + prefix_len),
            });
        }

        let end = self.current_position();

        Ok(Token::new(
            TokenKind::HereDoc { fd, strip_tabs, delimiter, body: String::new(), quoted },
            Span::new(start, end),
        ))
    }

    /// Read a heredoc delimiter. Quoted delimiters (`'EOF'`, `"EOF"`) disable expansion.
    fn read_heredoc_delimiter(&mut self) -> Result<(String, bool), LexerError> {
        let Some(&(start_pos, first_ch)) = self.chars.peek() else {
            return Ok((String::new(), false));
        };

        match first_ch {
            '\'' => self.read_single_quoted_delimiter(start_pos),
            '"' => self.read_double_quoted_delimiter(start_pos),
            _ => self.read_unquoted_delimiter(),
        }
    }

    fn read_single_quoted_delimiter(&mut self, start: usize) -> Result<(String, bool), LexerError> {
        self.chars.next();

        let mut delimiter = String::new();

        while let Some(&(pos, ch)) = self.chars.peek() {
            if ch == '\'' {
                self.chars.next();
                return Ok((delimiter, true));
            }
            if ch == '\n' || ch == '\r' {
                return Err(LexerError::UnterminatedSingleQuote { span: Span::new(start, pos) });
            }
            delimiter.push(ch);
            self.chars.next();
        }

        Err(LexerError::UnterminatedSingleQuote { span: Span::new(start, self.input.len()) })
    }

    fn read_double_quoted_delimiter(&mut self, start: usize) -> Result<(String, bool), LexerError> {
        self.chars.next();

        let mut delimiter = String::new();
        let mut escaped = false;

        while let Some(&(pos, ch)) = self.chars.peek() {
            if escaped {
                match ch {
                    '"' | '\\' | '$' | '`' => delimiter.push(ch),
                    '\n' => {} // line continuation
                    _ => {
                        delimiter.push('\\');
                        delimiter.push(ch);
                    }
                }
                escaped = false;
                self.chars.next();
                continue;
            }

            match ch {
                '"' => {
                    self.chars.next();
                    return Ok((delimiter, true));
                }
                '\\' => {
                    escaped = true;
                    self.chars.next();
                }
                '\n' | '\r' => {
                    return Err(LexerError::UnterminatedDoubleQuote {
                        span: Span::new(start, pos),
                    });
                }
                _ => {
                    delimiter.push(ch);
                    self.chars.next();
                }
            }
        }

        Err(LexerError::UnterminatedDoubleQuote { span: Span::new(start, self.input.len()) })
    }

    fn read_unquoted_delimiter(&mut self) -> Result<(String, bool), LexerError> {
        let mut delimiter = String::new();
        let mut escaped = false;

        while let Some(&(_, ch)) = self.chars.peek() {
            if escaped {
                delimiter.push(ch);
                escaped = false;
                self.chars.next();
                continue;
            }

            match ch {
                '\\' => {
                    escaped = true;
                    self.chars.next();
                }
                ' ' | '\t' | '\n' | '\r' | ';' | '&' | '|' | '(' | ')' | '<' | '>' => {
                    break;
                }
                _ => {
                    delimiter.push(ch);
                    self.chars.next();
                }
            }
        }

        Ok((delimiter, false))
    }

    fn lex_dup_target(
        &mut self,
        start: usize,
        source: u32,
        output: bool,
        prefix_len: usize,
    ) -> Result<Token, LexerError> {
        let target_start = self.chars.peek().map(|(pos, _)| *pos).unwrap_or(start + prefix_len);

        if self.peek_char() == Some('-') {
            self.chars.next();
            return Ok(Token::new(
                TokenKind::DuplicateFd { source, target: DupTarget::Close, output },
                Span::new(start, target_start + 1),
            ));
        }

        let mut end = target_start;
        while let Some(&(pos, ch)) = self.chars.peek() {
            if !ch.is_ascii_digit() {
                break;
            }
            end = pos + 1;
            self.chars.next();
        }

        if end == target_start {
            return Err(LexerError::InvalidRedirection {
                message: "expected file descriptor after >&".to_string(),
                span: Span::new(start, target_start),
            });
        }

        let target_fd: u32 =
            self.input[target_start..end].parse().map_err(|_| LexerError::InvalidRedirection {
                message: "invalid file descriptor".to_string(),
                span: Span::new(target_start, end),
            })?;

        Ok(Token::new(
            TokenKind::DuplicateFd { source, target: DupTarget::Fd(target_fd), output },
            Span::new(start, end),
        ))
    }
}
