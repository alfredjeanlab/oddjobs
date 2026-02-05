// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Utility methods for querying and analyzing AST nodes.

use super::{AstVisitor, BraceGroup, CommandList, SimpleCommand, Subshell, WordPart};

impl CommandList {
    /// Parse input string into a command list.
    ///
    /// This is a convenience wrapper around [`Parser::parse`].
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use oj_shell::CommandList;
    ///
    /// let ast = CommandList::parse("echo hello")?;
    /// assert_eq!(ast.count_simple_commands(), 1);
    /// # Ok::<(), oj_shell::ParseError>(())
    /// ```
    ///
    /// [`Parser::parse`]: super::super::Parser::parse
    pub fn parse(input: &str) -> Result<Self, super::super::parse_error::ParseError> {
        super::super::parser::Parser::parse(input)
    }

    /// Count the total number of simple commands in the AST.
    ///
    /// This includes commands in jobs, subshells, brace groups,
    /// and command substitutions.
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
    /// // Job counts each stage
    /// let ast = Parser::parse("cat file | grep pattern | wc -l")?;
    /// assert_eq!(ast.count_simple_commands(), 3);
    ///
    /// // Multiple commands separated by semicolon
    /// let ast = Parser::parse("echo a; echo b; echo c")?;
    /// assert_eq!(ast.count_simple_commands(), 3);
    ///
    /// // Commands in substitutions are counted too
    /// let ast = Parser::parse("echo $(cat file)")?;
    /// assert_eq!(ast.count_simple_commands(), 2); // echo + cat
    /// # Ok::<(), oj_shell::ParseError>(())
    /// ```
    pub fn count_simple_commands(&self) -> usize {
        struct Counter(usize);
        impl AstVisitor for Counter {
            fn visit_simple_command(&mut self, cmd: &SimpleCommand) {
                self.0 += 1;
                self.walk_simple_command(cmd);
            }
        }
        let mut counter = Counter(0);
        counter.visit_command_list(self);
        counter.0
    }

    /// Collect all variable names referenced in the AST.
    ///
    /// Returns a de-duplicated list of variable names in the order they
    /// first appear. This includes variables in double-quoted strings,
    /// unquoted expansions, and command substitutions.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use oj_shell::Parser;
    ///
    /// let ast = Parser::parse("echo $HOME and $USER")?;
    /// assert_eq!(ast.collect_variables(), vec!["HOME", "USER"]);
    ///
    /// // Duplicate variables appear only once
    /// let ast = Parser::parse("echo $PATH; export PATH=$PATH:/new")?;
    /// assert_eq!(ast.collect_variables(), vec!["PATH"]);
    ///
    /// // Variables in substitutions are collected
    /// let ast = Parser::parse("echo $(echo $VAR)")?;
    /// assert_eq!(ast.collect_variables(), vec!["VAR"]);
    /// # Ok::<(), oj_shell::ParseError>(())
    /// ```
    pub fn collect_variables(&self) -> Vec<String> {
        struct Collector(Vec<String>);
        impl AstVisitor for Collector {
            fn visit_word_part(&mut self, part: &WordPart) {
                if let WordPart::Variable { name, .. } = part {
                    if !self.0.contains(name) {
                        self.0.push(name.clone());
                    }
                }
                self.walk_word_part(part);
            }
        }
        let mut collector = Collector(Vec::new());
        collector.visit_command_list(self);
        collector.0
    }

    /// Check if the AST contains any command substitutions.
    ///
    /// Returns `true` if any `$(...)` or backtick substitutions are present
    /// anywhere in the AST, including nested inside other substitutions.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use oj_shell::Parser;
    ///
    /// // No substitutions
    /// let ast = Parser::parse("echo hello")?;
    /// assert!(!ast.has_command_substitutions());
    ///
    /// // Modern $(cmd) syntax
    /// let ast = Parser::parse("echo $(date)")?;
    /// assert!(ast.has_command_substitutions());
    ///
    /// // Backtick syntax
    /// let ast = Parser::parse("echo `date`")?;
    /// assert!(ast.has_command_substitutions());
    ///
    /// // Inside double quotes
    /// let ast = Parser::parse(r#"echo "Today is $(date)""#)?;
    /// assert!(ast.has_command_substitutions());
    /// # Ok::<(), oj_shell::ParseError>(())
    /// ```
    pub fn has_command_substitutions(&self) -> bool {
        struct Finder(bool);
        impl AstVisitor for Finder {
            fn visit_word_part(&mut self, part: &WordPart) {
                if matches!(part, WordPart::CommandSubstitution { .. }) {
                    self.0 = true;
                }
                self.walk_word_part(part);
            }
        }
        let mut finder = Finder(false);
        finder.visit_command_list(self);
        finder.0
    }

    /// Get the maximum nesting depth of subshells and brace groups.
    ///
    /// Returns 0 for a flat command list with no nested structures.
    /// This only counts subshells `(...)` and brace groups `{...}`, not
    /// command substitutions.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use oj_shell::Parser;
    ///
    /// // No nesting
    /// let ast = Parser::parse("echo hello | grep h")?;
    /// assert_eq!(ast.max_nesting_depth(), 0);
    ///
    /// // Single subshell
    /// let ast = Parser::parse("(echo hello)")?;
    /// assert_eq!(ast.max_nesting_depth(), 1);
    ///
    /// // Nested subshells
    /// let ast = Parser::parse("(echo a; (echo b; (echo c)))")?;
    /// assert_eq!(ast.max_nesting_depth(), 3);
    ///
    /// // Brace groups count too
    /// let ast = Parser::parse("{ echo a; { echo b; }; }")?;
    /// assert_eq!(ast.max_nesting_depth(), 2);
    /// # Ok::<(), oj_shell::ParseError>(())
    /// ```
    pub fn max_nesting_depth(&self) -> usize {
        struct DepthTracker {
            current: usize,
            max: usize,
        }
        impl AstVisitor for DepthTracker {
            fn visit_subshell(&mut self, subshell: &Subshell) {
                self.current += 1;
                self.max = self.max.max(self.current);
                self.walk_subshell(subshell);
                self.current -= 1;
            }
            fn visit_brace_group(&mut self, group: &BraceGroup) {
                self.current += 1;
                self.max = self.max.max(self.current);
                self.walk_brace_group(group);
                self.current -= 1;
            }
        }
        let mut tracker = DepthTracker { current: 0, max: 0 };
        tracker.visit_command_list(self);
        tracker.max
    }
}

#[cfg(test)]
#[path = "utils_tests.rs"]
mod tests;
