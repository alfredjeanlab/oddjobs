// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! CLI argument parsing utilities for shell commands.

use super::{SimpleCommand, Word, WordPart};

/// A parsed CLI argument from a command's argument list.
///
/// This categorizes shell words into typical CLI argument patterns:
/// flags, options with values, and positional arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliArg<'a> {
    /// A short flag like `-v` or `-abc` (multiple flags bundled).
    ShortFlag(&'a Word),
    /// A long flag like `--verbose` (no value).
    LongFlag(&'a Word),
    /// A long option with inline value: `--key=value`.
    LongOptionInline {
        /// The full word containing `--key=value`
        word: &'a Word,
        /// The key portion (e.g., "key")
        key: &'a str,
        /// The value portion (e.g., "value")
        value: &'a str,
    },
    /// A long option with separate value: `--key value`.
    LongOptionSeparate {
        /// The key word (`--key`)
        key_word: &'a Word,
        /// The key name (e.g., "key")
        key: &'a str,
        /// The value word
        value_word: &'a Word,
    },
    /// A positional argument (not a flag/option).
    Positional(&'a Word),
}

impl<'a> CliArg<'a> {
    /// Returns true if this is a flag (short or long, no value).
    pub fn is_flag(&self) -> bool {
        matches!(self, CliArg::ShortFlag(_) | CliArg::LongFlag(_))
    }

    /// Returns true if this is an option (with a value).
    pub fn is_option(&self) -> bool {
        matches!(
            self,
            CliArg::LongOptionInline { .. } | CliArg::LongOptionSeparate { .. }
        )
    }

    /// Returns true if this is a positional argument.
    pub fn is_positional(&self) -> bool {
        matches!(self, CliArg::Positional(_))
    }

    /// Returns the option key if this is a long option.
    pub fn option_key(&self) -> Option<&str> {
        match self {
            CliArg::LongOptionInline { key, .. } => Some(key),
            CliArg::LongOptionSeparate { key, .. } => Some(key),
            _ => None,
        }
    }
}

impl SimpleCommand {
    /// Parse the command's arguments into CLI argument categories.
    ///
    /// This interprets the argument list using common CLI conventions:
    /// - `-x` or `-abc`: short flags
    /// - `--flag`: long flag (not in `options_with_values`)
    /// - `--key=value`: long option with inline value
    /// - `--key value`: long option with separate value (if key is in `options_with_values`)
    /// - Everything else: positional arguments
    ///
    /// The `options_with_values` parameter specifies which long options consume
    /// the next argument as their value. Without this, we can't distinguish
    /// `--model haiku` (option with value) from `--print prompt` (flag + positional).
    ///
    /// # Example
    ///
    /// ```
    /// use oj_shell::{Parser, SimpleCommand, Command, CliArg};
    ///
    /// let ast = Parser::parse("cmd -v --model haiku --print pos1")?;
    /// let cmd = match &ast.commands[0].first.command {
    ///     Command::Simple(c) => c,
    ///     _ => panic!("expected simple command"),
    /// };
    ///
    /// let options_with_values = &["model"];
    /// let args = cmd.parse_cli_args(options_with_values);
    /// assert!(args[0].is_flag());       // -v
    /// assert!(args[1].is_option());     // --model haiku
    /// assert!(args[2].is_flag());       // --print
    /// assert!(args[3].is_positional()); // pos1
    /// # Ok::<(), oj_shell::ParseError>(())
    /// ```
    pub fn parse_cli_args(&self, options_with_values: &[&str]) -> Vec<CliArg<'_>> {
        let mut result = Vec::new();
        let mut i = 0;

        while i < self.args.len() {
            let word = &self.args[i];

            // Get the first literal part to check for flag/option patterns
            let first_literal = word.parts.first().and_then(|p| match p {
                WordPart::Literal { value, .. } => Some(value.as_str()),
                _ => None,
            });

            match first_literal {
                Some(s) if s.starts_with("--") => {
                    // Long option or flag
                    if let Some(eq_pos) = s.find('=') {
                        // --key=value (inline)
                        let key = &s[2..eq_pos];
                        let value = &s[eq_pos + 1..];
                        result.push(CliArg::LongOptionInline { word, key, value });
                    } else {
                        let key = &s[2..];
                        // Check if this option takes a value AND there's a next arg
                        let takes_value = options_with_values.contains(&key);
                        let has_next = i + 1 < self.args.len();

                        if takes_value && has_next {
                            // --key value (separate)
                            let value_word = &self.args[i + 1];
                            result.push(CliArg::LongOptionSeparate {
                                key_word: word,
                                key,
                                value_word,
                            });
                            i += 1; // Skip the value
                        } else {
                            // --flag (no value)
                            result.push(CliArg::LongFlag(word));
                        }
                    }
                }
                Some(s) if s.starts_with('-') && s.len() > 1 => {
                    // Short flag(s): -v, -abc, etc.
                    result.push(CliArg::ShortFlag(word));
                }
                _ => {
                    // Positional argument (or non-literal word like variable)
                    result.push(CliArg::Positional(word));
                }
            }

            i += 1;
        }

        result
    }

    /// Check if this command has an argument matching the given long option name.
    ///
    /// Matches both `--name` and `--name=...` forms.
    ///
    /// # Example
    ///
    /// ```
    /// use oj_shell::{Parser, Command};
    ///
    /// let ast = Parser::parse("cmd --session-id abc")?;
    /// let cmd = match &ast.commands[0].first.command {
    ///     Command::Simple(c) => c,
    ///     _ => panic!("expected simple command"),
    /// };
    /// assert!(cmd.has_long_option("session-id"));
    /// assert!(!cmd.has_long_option("verbose"));
    /// # Ok::<(), oj_shell::ParseError>(())
    /// ```
    pub fn has_long_option(&self, name: &str) -> bool {
        let with_eq = format!("--{}=", name);
        let exact = format!("--{}", name);

        self.args.iter().any(|word| {
            word.parts.first().is_some_and(|p| match p {
                WordPart::Literal { value, .. } => value == &exact || value.starts_with(&with_eq),
                _ => false,
            })
        })
    }

    /// Get all positional arguments (non-flag, non-option arguments).
    ///
    /// The `options_with_values` parameter specifies which long options consume
    /// the next argument as their value (see [`parse_cli_args`]).
    ///
    /// # Example
    ///
    /// ```
    /// use oj_shell::{Parser, Command};
    ///
    /// let ast = Parser::parse("cmd --flag pos1 --opt val pos2")?;
    /// let cmd = match &ast.commands[0].first.command {
    ///     Command::Simple(c) => c,
    ///     _ => panic!("expected simple command"),
    /// };
    /// let positionals = cmd.positional_args(&["opt"]);
    /// assert_eq!(positionals.len(), 2);  // pos1, pos2 (val is --opt's value)
    /// # Ok::<(), oj_shell::ParseError>(())
    /// ```
    pub fn positional_args(&self, options_with_values: &[&str]) -> Vec<&Word> {
        self.parse_cli_args(options_with_values)
            .into_iter()
            .filter_map(|arg| match arg {
                CliArg::Positional(w) => Some(w),
                _ => None,
            })
            .collect()
    }
}
