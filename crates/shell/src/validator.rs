// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! AST validator for shell commands.
//!
//! This module provides semantic validation for parsed shell ASTs,
//! checking for completeness and detecting common errors.

use super::ast::{
    AstVisitor, BraceGroup, Command, CommandItem, CommandList, Pipeline, SimpleCommand, Subshell,
    WordPart,
};
use super::token::Span;
pub use crate::validation::ValidationError;

/// Configuration for validation strictness.
#[derive(Debug, Clone)]
pub struct ValidatorConfig {
    /// Maximum allowed nesting depth (0 = unlimited).
    pub max_nesting_depth: usize,
    /// Whether standalone assignments are allowed (bash allows them).
    pub allow_standalone_assignments: bool,
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self {
            max_nesting_depth: 0,
            allow_standalone_assignments: true,
        }
    }
}

/// Validate a parsed command list for semantic correctness.
///
/// Returns `Ok(())` if the AST is valid, or `Err(errors)` with all
/// validation errors found.
///
/// # Example
///
/// ```
/// use oj_shell::{Parser, validate};
///
/// let ast = Parser::parse("echo hello").unwrap();
/// assert!(validate(&ast).is_ok());
///
/// let ast = Parser::parse("( )").unwrap();
/// assert!(validate(&ast).is_err());
/// ```
pub fn validate(ast: &CommandList) -> Result<(), Vec<ValidationError>> {
    validate_with_config(ast, ValidatorConfig::default())
}

/// Validate with custom configuration.
///
/// # Example
///
/// ```
/// use oj_shell::{Parser, validate_with_config, ValidatorConfig};
///
/// let config = ValidatorConfig {
///     max_nesting_depth: 3,
///     allow_standalone_assignments: false,
/// };
///
/// let ast = Parser::parse("echo hello").unwrap();
/// assert!(validate_with_config(&ast, config).is_ok());
/// ```
pub fn validate_with_config(
    ast: &CommandList,
    config: ValidatorConfig,
) -> Result<(), Vec<ValidationError>> {
    Validator::new(config).validate(ast)
}

/// Shell AST validator.
///
/// Uses the `AstVisitor` pattern to traverse the AST and collect validation errors.
struct Validator {
    config: ValidatorConfig,
    errors: Vec<ValidationError>,
    current_depth: usize,
}

impl Validator {
    fn new(config: ValidatorConfig) -> Self {
        Self {
            config,
            errors: Vec::new(),
            current_depth: 0,
        }
    }

    fn validate(mut self, ast: &CommandList) -> Result<(), Vec<ValidationError>> {
        self.visit_command_list(ast);
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors)
        }
    }

    fn report(&mut self, error: ValidationError) {
        self.errors.push(error);
    }

    fn check_nesting_depth(&mut self, span: Span) {
        if self.config.max_nesting_depth > 0 && self.current_depth > self.config.max_nesting_depth {
            self.report(ValidationError::ExcessiveNesting {
                depth: self.current_depth,
                max: self.config.max_nesting_depth,
                span,
            });
        }
    }

    /// Check if a SimpleCommand has an actual command (not just assignments).
    fn has_command_name(cmd: &SimpleCommand) -> bool {
        !cmd.name.parts.is_empty()
    }

    /// Extract a string representation of the first assignment value for error messages.
    fn assignment_value_str(cmd: &SimpleCommand) -> Option<String> {
        cmd.env.first().map(|env| {
            env.value
                .parts
                .iter()
                .map(|part| match part {
                    WordPart::Literal { value, .. } => value.clone(),
                    WordPart::Variable { name, .. } => format!("${name}"),
                    WordPart::CommandSubstitution { .. } => "$(...)".to_string(),
                })
                .collect::<String>()
        })
    }
}

impl AstVisitor for Validator {
    fn visit_command_list(&mut self, list: &CommandList) {
        // Empty command list is valid (empty input)
        self.walk_command_list(list);
    }

    fn visit_command_item(&mut self, item: &CommandItem) {
        // Check the underlying command
        self.visit_command(&item.command);
    }

    fn visit_command(&mut self, command: &Command) {
        self.walk_command(command);
    }

    fn visit_simple_command(&mut self, cmd: &SimpleCommand) {
        // Check for IFS assignments (not supported - we use fixed default word splitting)
        for env in &cmd.env {
            if env.name == "IFS" {
                self.report(ValidationError::IfsAssignment { span: cmd.span });
            }
        }

        // Check for command without name (only assignments)
        if !Self::has_command_name(cmd)
            && !cmd.env.is_empty()
            && !self.config.allow_standalone_assignments
        {
            if let Some(env) = cmd.env.first() {
                self.report(ValidationError::StandaloneAssignment {
                    name: env.name.clone(),
                    value: Self::assignment_value_str(cmd),
                    span: cmd.span,
                });
            }
        }
        self.walk_simple_command(cmd);
    }

    fn visit_pipeline(&mut self, pipeline: &Pipeline) {
        // Check for empty pipeline segments (commands with no name)
        for cmd in &pipeline.commands {
            if !Self::has_command_name(cmd) {
                self.report(ValidationError::EmptyPipelineSegment { span: cmd.span });
            }
        }
        self.walk_pipeline(pipeline);
    }

    fn visit_subshell(&mut self, subshell: &Subshell) {
        self.current_depth += 1;
        self.check_nesting_depth(subshell.span);

        if subshell.body.commands.is_empty() {
            self.report(ValidationError::EmptySubshell {
                span: subshell.span,
            });
        }

        self.walk_subshell(subshell);
        self.current_depth -= 1;
    }

    fn visit_brace_group(&mut self, group: &BraceGroup) {
        self.current_depth += 1;
        self.check_nesting_depth(group.span);

        if group.body.commands.is_empty() {
            self.report(ValidationError::EmptyBraceGroup { span: group.span });
        }

        self.walk_brace_group(group);
        self.current_depth -= 1;
    }
}

#[cfg(test)]
#[path = "validator_tests.rs"]
mod tests;
