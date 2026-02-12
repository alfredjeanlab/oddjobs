// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Semantic validation for parsed shell ASTs.

use super::ast::{
    AstVisitor, BraceGroup, Command, CommandItem, CommandList, Job, SimpleCommand, Subshell,
    WordPart,
};
use super::token::Span;
pub use crate::validation::ValidationError;

#[derive(Debug, Clone)]
pub struct ValidatorConfig {
    /// 0 = unlimited.
    pub max_nesting_depth: usize,
    /// Bash allows standalone assignments; POSIX does not.
    pub allow_standalone_assignments: bool,
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self { max_nesting_depth: 0, allow_standalone_assignments: true }
    }
}

pub fn validate(ast: &CommandList) -> Result<(), Vec<ValidationError>> {
    validate_with_config(ast, ValidatorConfig::default())
}

pub fn validate_with_config(
    ast: &CommandList,
    config: ValidatorConfig,
) -> Result<(), Vec<ValidationError>> {
    Validator::new(config).validate(ast)
}

struct Validator {
    config: ValidatorConfig,
    errors: Vec<ValidationError>,
    current_depth: usize,
}

impl Validator {
    fn new(config: ValidatorConfig) -> Self {
        Self { config, errors: Vec::new(), current_depth: 0 }
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

    fn has_command_name(cmd: &SimpleCommand) -> bool {
        !cmd.name.parts.is_empty()
    }

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
        self.walk_command_list(list);
    }

    fn visit_command_item(&mut self, item: &CommandItem) {
        self.visit_command(&item.command);
    }

    fn visit_command(&mut self, command: &Command) {
        self.walk_command(command);
    }

    fn visit_simple_command(&mut self, cmd: &SimpleCommand) {
        for env in &cmd.env {
            if env.name == "IFS" {
                self.report(ValidationError::IfsAssignment { span: cmd.span });
            }
        }

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

    fn visit_job(&mut self, job: &Job) {
        for cmd in &job.commands {
            if !Self::has_command_name(cmd) {
                self.report(ValidationError::EmptyJobSegment { span: cmd.span });
            }
        }
        self.walk_job(job);
    }

    fn visit_subshell(&mut self, subshell: &Subshell) {
        self.current_depth += 1;
        self.check_nesting_depth(subshell.span);

        if subshell.body.commands.is_empty() {
            self.report(ValidationError::EmptySubshell { span: subshell.span });
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
