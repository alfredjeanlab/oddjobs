// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shell lexer for tokenizing shell commands.

mod operators;
mod quotes;
mod redirection;
mod substitution;
mod variables;

use std::collections::VecDeque;

use super::token::{Span, Token, TokenKind};

pub use crate::error::LexerError;

/// Pending heredoc waiting for body capture.
struct PendingHereDoc {
    /// The delimiter word.
    delimiter: String,
    /// Whether to strip leading tabs.
    strip_tabs: bool,
    /// Index in the tokens vector to update.
    token_index: usize,
    /// Span of the heredoc operator (for error reporting).
    span: Span,
}

/// Track quote state during balanced content reading.
struct QuoteState {
    in_single_quote: bool,
    in_double_quote: bool,
    escaped: bool,
}

impl QuoteState {
    fn new() -> Self {
        Self {
            in_single_quote: false,
            in_double_quote: false,
            escaped: false,
        }
    }

    /// Process a character, updating quote state.
    /// Returns true if the character should be treated literally (inside quotes or escaped).
    fn process(&mut self, ch: char) -> bool {
        if self.escaped {
            self.escaped = false;
            return true;
        }
        match ch {
            '\\' if !self.in_single_quote => {
                self.escaped = true;
                false
            }
            '\'' if !self.in_double_quote => {
                self.in_single_quote = !self.in_single_quote;
                false
            }
            '"' if !self.in_single_quote => {
                self.in_double_quote = !self.in_double_quote;
                false
            }
            _ => self.in_single_quote || self.in_double_quote,
        }
    }
}

/// Shell lexer that tokenizes input into words, operators, and newlines.
pub struct Lexer<'a> {
    /// The input string being lexed.
    input: &'a str,
    /// Peekable iterator over character indices.
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    /// Pending heredocs awaiting body capture.
    pending_heredocs: VecDeque<PendingHereDoc>,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer for the given input.
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.char_indices().peekable(),
            pending_heredocs: VecDeque::new(),
        }
    }

    /// Peek at the next character without consuming it.
    #[inline]
    fn peek_char(&mut self) -> Option<char> {
        self.chars.peek().map(|(_, c)| *c)
    }

    /// Try to consume a line continuation (backslash followed by newline).
    ///
    /// Returns true if a line continuation was consumed.
    fn consume_line_continuation(&mut self) -> bool {
        let Some('\\') = self.peek_char() else {
            return false;
        };

        let mut lookahead = self.chars.clone();
        lookahead.next(); // skip backslash

        match lookahead.peek().map(|(_, c)| *c) {
            Some('\n') => {
                self.chars.next(); // consume backslash
                self.chars.next(); // consume \n
                true
            }
            Some('\r') => {
                lookahead.next();
                if lookahead.peek().map(|(_, c)| *c) == Some('\n') {
                    self.chars.next(); // consume backslash
                    self.chars.next(); // consume \r
                    self.chars.next(); // consume \n
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Consume a newline (LF or CRLF), returning the byte length consumed.
    ///
    /// Returns 0 if not at a newline.
    fn consume_newline(&mut self) -> usize {
        match self.peek_char() {
            Some('\n') => {
                self.chars.next();
                1
            }
            Some('\r') => {
                self.chars.next();
                if self.peek_char() == Some('\n') {
                    self.chars.next();
                    2
                } else {
                    1
                }
            }
            _ => 0,
        }
    }

    /// Tokenize the entire input and return a vector of tokens.
    pub fn tokenize(input: &str) -> Result<Vec<Token>, LexerError> {
        let mut lexer = Lexer::new(input);
        // Pre-allocate based on heuristic: ~1 token per 4 characters
        let mut tokens = Vec::with_capacity(input.len() / 4 + 1);

        while let Some(token) = lexer.next_token()? {
            let is_newline = matches!(token.kind, TokenKind::Newline);

            // Track heredocs for deferred body capture
            if let TokenKind::HereDoc {
                ref delimiter,
                strip_tabs,
                ..
            } = token.kind
            {
                lexer.pending_heredocs.push_back(PendingHereDoc {
                    delimiter: delimiter.clone(),
                    strip_tabs,
                    token_index: tokens.len(),
                    span: token.span,
                });
            }

            tokens.push(token);

            // On newline, capture any pending heredoc bodies
            if is_newline && !lexer.pending_heredocs.is_empty() {
                lexer.capture_pending_heredocs(&mut tokens)?;
            }
        }

        // Check for unterminated heredocs
        if let Some(pending) = lexer.pending_heredocs.front() {
            return Err(LexerError::UnterminatedHereDoc {
                delimiter: pending.delimiter.clone(),
                span: pending.span,
            });
        }

        Ok(tokens)
    }

    /// Get the current position in the input.
    fn current_position(&self) -> usize {
        self.chars
            .clone()
            .next()
            .map(|(pos, _)| pos)
            .unwrap_or(self.input.len())
    }

    /// Get the next token, or None if at end of input.
    fn next_token(&mut self) -> Result<Option<Token>, LexerError> {
        // Skip whitespace (but not newlines)
        self.skip_whitespace();

        let Some(&(pos, ch)) = self.chars.peek() else {
            return Ok(None);
        };

        match ch {
            '\n' => Ok(Some(self.lex_newline(pos))),
            '\r' => {
                // Handle \r or \r\n as newline (don't collapse - heredocs need line structure)
                let len = self.consume_newline();
                Ok(Some(Token::new(
                    TokenKind::Newline,
                    Span::new(pos, pos + len),
                )))
            }
            '&' => Ok(Some(self.lex_ampersand(pos))),
            '|' => Ok(Some(self.lex_pipe(pos))),
            ';' => {
                self.chars.next();
                Ok(Some(Token::new(TokenKind::Semi, Span::new(pos, pos + 1))))
            }
            '(' => {
                self.chars.next();
                Ok(Some(Token::new(TokenKind::LParen, Span::new(pos, pos + 1))))
            }
            ')' => {
                self.chars.next();
                Ok(Some(Token::new(TokenKind::RParen, Span::new(pos, pos + 1))))
            }
            '{' => {
                self.chars.next();
                Ok(Some(Token::new(TokenKind::LBrace, Span::new(pos, pos + 1))))
            }
            '}' => {
                self.chars.next();
                Ok(Some(Token::new(TokenKind::RBrace, Span::new(pos, pos + 1))))
            }
            '$' => Ok(Some(self.lex_variable(pos)?)),
            '`' => Ok(Some(self.lex_backtick_substitution(pos)?)),
            '>' => Ok(Some(self.lex_redirect_out(pos, None)?)),
            '<' => Ok(Some(self.lex_redirect_in(pos, None)?)),
            '\'' => Ok(Some(self.lex_single_quote(pos)?)),
            '"' => Ok(Some(self.lex_double_quote(pos)?)),
            _ => Ok(Some(self.lex_word(pos)?)),
        }
    }

    /// Skip whitespace characters (space, tab, and line continuations).
    ///
    /// Line continuation is backslash followed by newline (LF or CRLF).
    /// This makes `echo hello \<newline>world` equivalent to `echo hello world`.
    fn skip_whitespace(&mut self) {
        loop {
            match self.peek_char() {
                Some(' ' | '\t') => {
                    self.chars.next();
                }
                Some('\\') if self.consume_line_continuation() => {}
                _ => break,
            }
        }
    }

    /// Lex a word token (command, argument, etc.).
    ///
    /// Called when peek() has confirmed there is a character that isn't a special token.
    /// Also handles file descriptor prefixes (e.g., `2>` becomes RedirectOut with fd=2).
    /// Handles line continuations (backslash-newline) within words.
    fn lex_word(&mut self, start: usize) -> Result<Token, LexerError> {
        let mut word = String::new();
        let mut end = start;

        while let Some(&(pos, ch)) = self.chars.peek() {
            if self.consume_line_continuation() {
                continue;
            }
            if ch == '\\' {
                // Backslash escaping in unquoted context: the next character
                // is treated as a literal regardless of its special meaning.
                // This handles \( \) \; \| \& \\ \<space> etc.
                let mut lookahead = self.chars.clone();
                lookahead.next(); // skip backslash
                if let Some(&(next_pos, next_ch)) = lookahead.peek() {
                    // Not a newline (line continuation is handled above)
                    if next_ch != '\n' && next_ch != '\r' {
                        self.chars.next(); // consume backslash
                        self.chars.next(); // consume escaped character

                        // For glob metacharacters, preserve the backslash so the
                        // expansion phase can distinguish escaped from unescaped.
                        // The expansion phase will strip the backslash and mark
                        // the character as ineligible for glob expansion.
                        if matches!(next_ch, '*' | '?' | '[') {
                            word.push('\\');
                        }
                        word.push(next_ch);
                        end = next_pos + next_ch.len_utf8();
                        continue;
                    }
                }
                // Trailing backslash at EOF or before newline — treat as literal
                word.push(ch);
                end = pos + ch.len_utf8();
                self.chars.next();
                continue;
            }
            if Self::is_word_boundary(ch) {
                break;
            }
            word.push(ch);
            end = pos + ch.len_utf8();
            self.chars.next();
        }

        // Check if this word is a file descriptor followed by redirection
        if let Some(next_ch) = self.peek_char() {
            if (next_ch == '<' || next_ch == '>') && word.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(fd) = word.parse::<u32>() {
                    return match next_ch {
                        '>' => self.lex_redirect_out(start, Some(fd)),
                        '<' => self.lex_redirect_in(start, Some(fd)),
                        _ => unreachable!(),
                    };
                }
            }
        }

        Ok(Token::new(TokenKind::Word(word), Span::new(start, end)))
    }

    /// Check if a character is a word boundary.
    #[inline]
    fn is_word_boundary(ch: char) -> bool {
        matches!(
            ch,
            ' ' | '\t'
                | '\n'
                | '\r'
                | '&'
                | '|'
                | ';'
                | '$'
                | '`'
                | '<'
                | '>'
                | '('
                | ')'
                | '{'
                | '}'
                | '\''
                | '"'
        )
    }
}

#[cfg(test)]
#[path = "../lexer_tests/mod.rs"]
mod tests;
