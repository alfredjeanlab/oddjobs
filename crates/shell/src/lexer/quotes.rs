// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Quote handling (single and double quoted strings).

use super::{Lexer, LexerError};
use crate::ast::{SubstitutionBody, WordPart};
use crate::token::{Span, Token, TokenKind};

impl Lexer<'_> {
    pub(super) fn lex_single_quote(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next();
        let content_start = start + 1;
        let mut content_end = content_start;
        while let Some(&(pos, ch)) = self.chars.peek() {
            if ch == '\'' {
                let content = self.input[content_start..content_end].to_string();
                self.chars.next();
                return Ok(Token::new(TokenKind::SingleQuoted(content), Span::new(start, pos + 1)));
            }
            content_end = pos + ch.len_utf8();
            self.chars.next();
        }
        Err(LexerError::UnterminatedSingleQuote { span: Span::new(start, content_end) })
    }

    /// Emits empty boundary literals when the string starts or ends with
    /// a non-literal (variable/substitution) to preserve quote context in the AST.
    pub(super) fn lex_double_quote(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next();
        let mut parts: Vec<WordPart> = Vec::new();
        let mut current_literal = String::new();
        let mut last_pos = start + 1;
        let mut last_was_expansion = false;

        while let Some(&(pos, ch)) = self.chars.peek() {
            last_pos = pos + ch.len_utf8();
            match ch {
                '"' => {
                    if !current_literal.is_empty() || last_was_expansion {
                        parts.push(WordPart::double_quoted(current_literal));
                    }
                    self.chars.next();
                    return Ok(Token::new(
                        TokenKind::DoubleQuoted(parts),
                        Span::new(start, pos + 1),
                    ));
                }
                '\\' => {
                    self.chars.next();
                    let Some(&(esc_pos, esc_ch)) = self.chars.peek() else {
                        return Err(LexerError::TrailingBackslash {
                            span: Span::new(pos, pos + 1),
                        });
                    };
                    last_pos = esc_pos + esc_ch.len_utf8();
                    let replacement = match esc_ch {
                        '\\' => '\\',
                        'n' => '\n',
                        't' => '\t',
                        '"' => '"',
                        '\'' => '\'',
                        '$' => '$',
                        '`' => '`',
                        _ => {
                            return Err(LexerError::InvalidEscape {
                                ch: esc_ch,
                                span: Span::new(pos, last_pos),
                            })
                        }
                    };
                    current_literal.push(replacement);
                    self.chars.next();
                    last_was_expansion = false;
                }
                '$' => {
                    if !current_literal.is_empty() || parts.is_empty() {
                        parts.push(WordPart::double_quoted(std::mem::take(&mut current_literal)));
                    }
                    let part = self.lex_quoted_dollar(pos)?;
                    parts.push(part);
                    last_was_expansion = true;
                }
                '`' => {
                    if !current_literal.is_empty() || parts.is_empty() {
                        parts.push(WordPart::double_quoted(std::mem::take(&mut current_literal)));
                    }
                    let part = self.lex_quoted_backtick(pos)?;
                    parts.push(part);
                    last_was_expansion = true;
                }
                _ => {
                    current_literal.push(ch);
                    self.chars.next();
                    last_was_expansion = false;
                }
            }
        }
        Err(LexerError::UnterminatedDoubleQuote { span: Span::new(start, last_pos) })
    }

    fn lex_quoted_dollar(&mut self, start: usize) -> Result<WordPart, LexerError> {
        self.chars.next();

        let Some(&(name_start, ch)) = self.chars.peek() else {
            return Ok(WordPart::double_quoted("$"));
        };

        match ch {
            '(' => {
                self.chars.next();
                let content = self.read_balanced_content('(', ')', start)?;
                Ok(WordPart::CommandSubstitution {
                    body: SubstitutionBody::Unparsed(content),
                    backtick: false,
                })
            }
            '{' => {
                self.chars.next();
                self.lex_quoted_braced_variable(start)
            }
            _ if Self::is_valid_variable_start(ch) => {
                let name = self.scan_variable_name(name_start);
                Ok(WordPart::Variable { name, modifier: None })
            }
            _ => Ok(WordPart::double_quoted("$")),
        }
    }

    fn lex_quoted_braced_variable(&mut self, start: usize) -> Result<WordPart, LexerError> {
        let var = self.parse_braced_variable(start)?;
        Ok(WordPart::Variable { name: var.name, modifier: var.modifier })
    }

    fn lex_quoted_backtick(&mut self, start: usize) -> Result<WordPart, LexerError> {
        self.chars.next();

        let content = self.read_backtick_content(start)?;
        Ok(WordPart::CommandSubstitution {
            body: SubstitutionBody::Unparsed(content),
            backtick: true,
        })
    }
}
