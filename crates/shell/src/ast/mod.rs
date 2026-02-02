// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Abstract Syntax Tree types for parsed shell commands.

mod args;
mod utils;
mod visitor;

pub use args::CliArg;
pub use visitor::AstVisitor;

use super::token::Span;

/// A command list containing one or more command chains separated by `;` or newlines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandList {
    /// The command chains in this list.
    pub commands: Vec<AndOrList>,
    /// Source span covering the entire list.
    pub span: Span,
}

/// A chain of commands connected by `&&` or `||`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndOrList {
    /// First command in the chain.
    pub first: CommandItem,
    /// Rest of the chain: (operator, command) pairs.
    pub rest: Vec<(LogicalOp, CommandItem)>,
    /// Source span covering the entire chain.
    pub span: Span,
}

/// Logical operator for AND/OR chains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    /// `&&` - execute next if previous succeeded
    And,
    /// `||` - execute next if previous failed
    Or,
}

/// A command or pipeline with optional background execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandItem {
    /// The command (simple or pipeline).
    pub command: Command,
    /// True if command should run in background (`&`).
    pub background: bool,
    /// Source span.
    pub span: Span,
}

/// A single command in the AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// A simple command (command name with arguments).
    Simple(SimpleCommand),
    /// A pipeline of commands connected by `|`.
    Pipeline(Pipeline),
    /// A subshell: `(command_list)`.
    Subshell(Subshell),
    /// A brace group: `{ command_list; }`.
    BraceGroup(BraceGroup),
}

impl Command {
    /// Returns the span covering the entire command.
    pub fn span(&self) -> Span {
        match self {
            Command::Simple(c) => c.span,
            Command::Pipeline(p) => p.span,
            Command::Subshell(s) => s.span,
            Command::BraceGroup(b) => b.span,
        }
    }
}

/// A pipeline of commands connected by `|`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    /// Commands in the pipeline (at least 2).
    pub commands: Vec<SimpleCommand>,
    /// Source span covering the entire pipeline.
    pub span: Span,
}

/// A subshell executes commands in a child shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subshell {
    /// The commands to execute in the subshell.
    pub body: Box<CommandList>,
    /// Redirections attached to this subshell.
    pub redirections: Vec<Redirection>,
    /// Source span including parentheses.
    pub span: Span,
}

/// A brace group executes commands in the current shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BraceGroup {
    /// The commands to execute in the group.
    pub body: Box<CommandList>,
    /// Redirections attached to this brace group.
    pub redirections: Vec<Redirection>,
    /// Source span including braces.
    pub span: Span,
}

/// An environment variable assignment prefix.
///
/// Used for `VAR=value` prefixes in commands like `FOO=bar cmd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvAssignment {
    /// The variable name.
    pub name: String,
    /// The assigned value (as a Word to support future expansion).
    pub value: Word,
    /// Source span for this assignment.
    pub span: Span,
}

/// A redirection attached to a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirection {
    /// Output redirection: `>` or `>>` or `2>` etc.
    Out {
        /// File descriptor (None = stdout/1)
        fd: Option<u32>,
        /// Target file or word to redirect to
        target: Word,
        /// True for append (`>>`), false for overwrite (`>`)
        append: bool,
    },
    /// Input redirection: `<`
    In {
        /// File descriptor (None = stdin/0)
        fd: Option<u32>,
        /// Source file to redirect from
        source: Word,
    },
    /// Here-document: `<<DELIM` or `<<-DELIM`
    HereDoc {
        /// File descriptor (None = stdin/0)
        fd: Option<u32>,
        /// Delimiter string
        delimiter: String,
        /// Document body
        body: String,
        /// True if leading tabs should be stripped (`<<-`)
        strip_tabs: bool,
        /// True if delimiter was quoted (suppresses expansion)
        quoted: bool,
    },
    /// Here-string: `<<<`
    HereString {
        /// File descriptor (None = stdin/0)
        fd: Option<u32>,
        /// Content to provide as input
        content: Word,
    },
    /// Combined stderr+stdout redirection: `&>` or `&>>`
    Both {
        /// True for append (`&>>`), false for overwrite (`&>`)
        append: bool,
        /// Target file to redirect to
        target: Word,
    },
    /// File descriptor duplication: `n>&m`, `n<&m`, or `n>&-` / `n<&-`
    Duplicate {
        /// Source file descriptor
        source: u32,
        /// Target file descriptor or close
        target: super::token::DupTarget,
        /// True for output dup (`>&`), false for input dup (`<&`)
        output: bool,
    },
}

impl Redirection {
    /// Span of the trailing word (target/source/content), if any.
    pub fn target_span(&self) -> Option<Span> {
        match self {
            Redirection::Out { target, .. } => Some(target.span),
            Redirection::In { source, .. } => Some(source.span),
            Redirection::HereString { content, .. } => Some(content.span),
            Redirection::Both { target, .. } => Some(target.span),
            Redirection::HereDoc { .. } => None,
            Redirection::Duplicate { .. } => None,
        }
    }
}

/// A simple command: optional env assignments, a command name, and arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    /// Environment variable assignments that prefix the command.
    pub env: Vec<EnvAssignment>,
    /// The command name (first word after assignments).
    pub name: Word,
    /// Command arguments (remaining words).
    pub args: Vec<Word>,
    /// Redirections attached to this command.
    pub redirections: Vec<Redirection>,
    /// Source span covering the entire command.
    pub span: Span,
}

/// A word in a command (can be literal, variable, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    /// The parts that make up this word.
    pub parts: Vec<WordPart>,
    /// Source span for this word.
    pub span: Span,
}

/// Quoting style for literal text in the AST.
///
/// This information is preserved from parsing to support:
/// - Glob suppression: quoted strings should not undergo glob expansion
/// - Variable expansion: `'$VAR'` is literal, `"$VAR"` expands, `$VAR` expands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuoteStyle {
    /// Unquoted literal (subject to glob expansion and word splitting).
    #[default]
    Unquoted,
    /// Single-quoted literal (no expansion, no glob).
    Single,
    /// Double-quoted literal (variable/command expansion enabled, no glob).
    Double,
}

/// Command substitution body — unparsed at the token level, parsed in the AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubstitutionBody {
    /// Unparsed source text (as produced by the lexer).
    Unparsed(String),
    /// Parsed AST (as produced by the parser).
    Parsed(Box<CommandList>),
}

/// A part of a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordPart {
    /// Literal text with quoting information.
    Literal {
        /// The text content.
        value: String,
        /// How this literal was quoted in the source.
        quoted: QuoteStyle,
    },
    /// Variable reference.
    Variable {
        /// The variable name.
        name: String,
        /// Optional modifier (e.g., `:-default`).
        modifier: Option<String>,
    },
    /// Command substitution.
    CommandSubstitution {
        /// The substitution body (unparsed at token level, parsed in AST).
        body: SubstitutionBody,
        /// True if backtick syntax was used.
        backtick: bool,
    },
}

impl WordPart {
    /// Create an unquoted literal.
    pub fn literal(value: impl Into<String>) -> Self {
        WordPart::Literal {
            value: value.into(),
            quoted: QuoteStyle::Unquoted,
        }
    }

    /// Create a single-quoted literal.
    pub fn single_quoted(value: impl Into<String>) -> Self {
        WordPart::Literal {
            value: value.into(),
            quoted: QuoteStyle::Single,
        }
    }

    /// Create a double-quoted literal.
    pub fn double_quoted(value: impl Into<String>) -> Self {
        WordPart::Literal {
            value: value.into(),
            quoted: QuoteStyle::Double,
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
