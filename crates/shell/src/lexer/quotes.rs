// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Quote handling (single and double quoted strings).

use super::{Lexer, LexerError};
use crate::ast::{SubstitutionBody, WordPart};
use crate::token::{Span, Token, TokenKind};

impl Lexer<'_> {
    /// Lex a single-quoted string. Content is preserved literally with no escape processing.
    pub(super) fn lex_single_quote(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next(); // consume opening '
        let content_start = start + 1;
        let mut content_end = content_start;
        while let Some(&(pos, ch)) = self.chars.peek() {
            if ch == '\'' {
                let content = self.input[content_start..content_end].to_string();
                self.chars.next(); // consume closing '
                return Ok(Token::new(
                    TokenKind::SingleQuoted(content),
                    Span::new(start, pos + 1),
                ));
            }
            content_end = pos + ch.len_utf8();
            self.chars.next();
        }
        Err(LexerError::UnterminatedSingleQuote {
            span: Span::new(start, content_end),
        })
    }

    /// Lex a double-quoted string. Processes escapes (`\\`, `\n`, `\t`, `\"`, `\'`).
    /// Variable references and command substitutions are parsed into separate parts.
    ///
    /// For word splitting support, we emit empty boundary literals when the string
    /// starts or ends with a non-literal (variable/substitution). This preserves
    /// quote context in the AST.
    pub(super) fn lex_double_quote(&mut self, start: usize) -> Result<Token, LexerError> {
        self.chars.next(); // consume opening "
        let mut parts: Vec<WordPart> = Vec::new();
        let mut current_literal = String::new();
        let mut last_pos = start + 1;
        // Track whether the last thing added was a non-literal (variable/substitution)
        let mut last_was_expansion = false;

        while let Some(&(pos, ch)) = self.chars.peek() {
            last_pos = pos + ch.len_utf8();
            match ch {
                '"' => {
                    // Flush any pending literal, or emit boundary literal if we just had an expansion
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
                        '$' => '$', // Allow escaping $ in double quotes
                        '`' => '`', // Allow escaping backtick in double quotes
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
                    // Flush any pending literal, or emit boundary literal if starting with expansion
                    if !current_literal.is_empty() || parts.is_empty() {
                        parts.push(WordPart::double_quoted(std::mem::take(
                            &mut current_literal,
                        )));
                    }
                    // Parse variable or command substitution
                    let part = self.lex_quoted_dollar(pos)?;
                    parts.push(part);
                    last_was_expansion = true;
                }
                '`' => {
                    // Flush any pending literal, or emit boundary literal if starting with expansion
                    if !current_literal.is_empty() || parts.is_empty() {
                        parts.push(WordPart::double_quoted(std::mem::take(
                            &mut current_literal,
                        )));
                    }
                    // Parse backtick command substitution
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
        Err(LexerError::UnterminatedDoubleQuote {
            span: Span::new(start, last_pos),
        })
    }

    /// Lex a `$` inside a double-quoted string (variable or command substitution).
    fn lex_quoted_dollar(&mut self, start: usize) -> Result<WordPart, LexerError> {
        self.chars.next(); // consume $

        let Some(&(name_start, ch)) = self.chars.peek() else {
            // $ at end of string - treat as literal
            return Ok(WordPart::double_quoted("$"));
        };

        match ch {
            '(' => {
                self.chars.next(); // consume (
                let content = self.read_balanced_content('(', ')', start)?;
                Ok(WordPart::CommandSubstitution {
                    body: SubstitutionBody::Unparsed(content),
                    backtick: false,
                })
            }
            '{' => {
                self.chars.next(); // consume {
                self.lex_quoted_braced_variable(start)
            }
            _ if Self::is_valid_variable_start(ch) => {
                let name = self.scan_variable_name(name_start);
                Ok(WordPart::Variable {
                    name,
                    modifier: None,
                })
            }
            _ => {
                // Not a variable - emit $ as literal
                Ok(WordPart::double_quoted("$"))
            }
        }
    }

    /// Lex a braced variable inside a double-quoted string.
    fn lex_quoted_braced_variable(&mut self, start: usize) -> Result<WordPart, LexerError> {
        let var = self.parse_braced_variable(start)?;
        Ok(WordPart::Variable {
            name: var.name,
            modifier: var.modifier,
        })
    }

    /// Lex a backtick command substitution inside a double-quoted string.
    fn lex_quoted_backtick(&mut self, start: usize) -> Result<WordPart, LexerError> {
        self.chars.next(); // consume opening `

        let content = self.read_backtick_content(start)?;
        Ok(WordPart::CommandSubstitution {
            body: SubstitutionBody::Unparsed(content),
            backtick: true,
        })
    }
}
