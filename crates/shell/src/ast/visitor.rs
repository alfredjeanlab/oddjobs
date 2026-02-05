// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Visitor pattern for traversing the AST.

use super::{
    AndOrList, BraceGroup, Command, CommandItem, CommandList, EnvAssignment, Job, Redirection,
    SimpleCommand, Subshell, SubstitutionBody, Word, WordPart,
};

/// Visitor trait for traversing the AST.
///
/// This trait provides default implementations that walk the entire tree.
/// Override specific methods to perform custom operations at each node type.
///
/// Each `visit_*` method has a corresponding `walk_*` method. The `visit_*`
/// method is called at a node, and can call `walk_*` to descend into children.
/// To stop traversal at a node, simply don't call `walk_*`.
///
/// # Example: Count Variables
///
/// ```ignore
/// use oj_shell::{Parser, AstVisitor, CommandList, WordPart};
///
/// struct VariableCounter(usize);
///
/// impl AstVisitor for VariableCounter {
///     fn visit_word_part(&mut self, part: &WordPart) {
///         if matches!(part, WordPart::Variable { .. }) {
///             self.0 += 1;
///         }
///         // Continue traversal into command substitutions
///         self.walk_word_part(part);
///     }
/// }
///
/// let ast = Parser::parse("echo $HOME to $USER")?;
/// let mut counter = VariableCounter(0);
/// counter.visit_command_list(&ast);
/// assert_eq!(counter.0, 2);
/// # Ok::<(), oj_shell::ParseError>(())
/// ```
///
/// # Example: Find Command Names
///
/// ```ignore
/// use oj_shell::{Parser, AstVisitor, SimpleCommand, WordPart};
///
/// struct CommandFinder(Vec<String>);
///
/// impl AstVisitor for CommandFinder {
///     fn visit_simple_command(&mut self, cmd: &SimpleCommand) {
///         // Extract the command name if it's a simple literal
///         if let Some(WordPart::Literal { value, .. }) = cmd.name.parts.first() {
///             self.0.push(value.clone());
///         }
///         // Walk children to find commands in substitutions
///         self.walk_simple_command(cmd);
///     }
/// }
///
/// let ast = Parser::parse("echo $(cat file | grep pattern)")?;
/// let mut finder = CommandFinder(Vec::new());
/// finder.visit_command_list(&ast);
/// assert_eq!(finder.0, vec!["echo", "cat", "grep"]);
/// # Ok::<(), oj_shell::ParseError>(())
/// ```
pub trait AstVisitor {
    /// Visit a command list.
    fn visit_command_list(&mut self, cmd_list: &CommandList) {
        self.walk_command_list(cmd_list);
    }

    /// Visit an and-or list.
    fn visit_and_or_list(&mut self, and_or: &AndOrList) {
        self.walk_and_or_list(and_or);
    }

    /// Visit a command item.
    fn visit_command_item(&mut self, item: &CommandItem) {
        self.walk_command_item(item);
    }

    /// Visit a command.
    fn visit_command(&mut self, command: &Command) {
        self.walk_command(command);
    }

    /// Visit a simple command.
    fn visit_simple_command(&mut self, cmd: &SimpleCommand) {
        self.walk_simple_command(cmd);
    }

    /// Visit a job.
    fn visit_job(&mut self, job: &Job) {
        self.walk_job(job);
    }

    /// Visit a subshell.
    fn visit_subshell(&mut self, subshell: &Subshell) {
        self.walk_subshell(subshell);
    }

    /// Visit a brace group.
    fn visit_brace_group(&mut self, group: &BraceGroup) {
        self.walk_brace_group(group);
    }

    /// Visit a word.
    fn visit_word(&mut self, word: &Word) {
        self.walk_word(word);
    }

    /// Visit a word part.
    fn visit_word_part(&mut self, part: &WordPart) {
        self.walk_word_part(part);
    }

    /// Visit an environment assignment.
    fn visit_env_assignment(&mut self, assignment: &EnvAssignment) {
        self.walk_env_assignment(assignment);
    }

    /// Visit a redirection.
    fn visit_redirection(&mut self, redir: &Redirection) {
        self.walk_redirection(redir);
    }

    // Default walk implementations

    /// Walk a command list, visiting all and-or lists.
    fn walk_command_list(&mut self, cmd_list: &CommandList) {
        for and_or in &cmd_list.commands {
            self.visit_and_or_list(and_or);
        }
    }

    /// Walk an and-or list, visiting the first and rest items.
    fn walk_and_or_list(&mut self, and_or: &AndOrList) {
        self.visit_command_item(&and_or.first);
        for (_, item) in &and_or.rest {
            self.visit_command_item(item);
        }
    }

    /// Walk a command item, visiting the command.
    fn walk_command_item(&mut self, item: &CommandItem) {
        self.visit_command(&item.command);
    }

    /// Walk a command, dispatching to the appropriate variant.
    fn walk_command(&mut self, command: &Command) {
        match command {
            Command::Simple(cmd) => self.visit_simple_command(cmd),
            Command::Job(p) => self.visit_job(p),
            Command::Subshell(s) => self.visit_subshell(s),
            Command::BraceGroup(b) => self.visit_brace_group(b),
        }
    }

    /// Walk a simple command, visiting env assignments, name, arguments, and redirections.
    fn walk_simple_command(&mut self, cmd: &SimpleCommand) {
        for env in &cmd.env {
            self.visit_env_assignment(env);
        }
        self.visit_word(&cmd.name);
        for arg in &cmd.args {
            self.visit_word(arg);
        }
        for redir in &cmd.redirections {
            self.visit_redirection(redir);
        }
    }

    /// Walk an environment assignment, visiting the value.
    fn walk_env_assignment(&mut self, assignment: &EnvAssignment) {
        self.visit_word(&assignment.value);
    }

    /// Walk a redirection, visiting target/source words.
    fn walk_redirection(&mut self, redir: &Redirection) {
        match redir {
            Redirection::Out { target, .. } => self.visit_word(target),
            Redirection::In { source, .. } => self.visit_word(source),
            Redirection::HereString { content, .. } => self.visit_word(content),
            Redirection::Both { target, .. } => self.visit_word(target),
            Redirection::HereDoc { .. } => {
                // HereDoc body is pre-parsed string, no words to visit
            }
            Redirection::Duplicate { .. } => {
                // No child words to visit
            }
        }
    }

    /// Walk a job, visiting all commands.
    fn walk_job(&mut self, job: &Job) {
        for cmd in &job.commands {
            self.visit_simple_command(cmd);
        }
    }

    /// Walk a subshell, visiting the body and redirections.
    fn walk_subshell(&mut self, subshell: &Subshell) {
        self.visit_command_list(&subshell.body);
        for redir in &subshell.redirections {
            self.visit_redirection(redir);
        }
    }

    /// Walk a brace group, visiting the body and redirections.
    fn walk_brace_group(&mut self, group: &BraceGroup) {
        self.visit_command_list(&group.body);
        for redir in &group.redirections {
            self.visit_redirection(redir);
        }
    }

    /// Walk a word, visiting all parts.
    fn walk_word(&mut self, word: &Word) {
        for part in &word.parts {
            self.visit_word_part(part);
        }
    }

    /// Walk a word part, recursing into command substitutions.
    fn walk_word_part(&mut self, part: &WordPart) {
        if let WordPart::CommandSubstitution {
            body: SubstitutionBody::Parsed(body),
            ..
        } = part
        {
            self.visit_command_list(body);
        }
    }
}
