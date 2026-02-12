// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Abstract Syntax Tree types for parsed shell commands.

use super::token::Span;

/// One or more command chains separated by `;` or newlines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandList {
    pub commands: Vec<AndOrList>,
    pub span: Span,
}

/// Commands connected by `&&` or `||`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndOrList {
    pub first: CommandItem,
    pub rest: Vec<(LogicalOp, CommandItem)>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    /// `&&`
    And,
    /// `||`
    Or,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandItem {
    pub command: Command,
    /// True if command should run in background (`&`).
    pub background: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Simple(SimpleCommand),
    /// Commands connected by `|`.
    Job(Job),
    /// `(command_list)`
    Subshell(Subshell),
    /// `{ command_list; }`
    BraceGroup(BraceGroup),
}

impl Command {
    pub fn span(&self) -> Span {
        match self {
            Command::Simple(c) => c.span,
            Command::Job(p) => p.span,
            Command::Subshell(s) => s.span,
            Command::BraceGroup(b) => b.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// At least 2 commands connected by `|`.
    pub commands: Vec<SimpleCommand>,
    pub span: Span,
}

/// Executes commands in a child shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subshell {
    pub body: Box<CommandList>,
    pub redirections: Vec<Redirection>,
    pub span: Span,
}

/// Executes commands in the current shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BraceGroup {
    pub body: Box<CommandList>,
    pub redirections: Vec<Redirection>,
    pub span: Span,
}

/// `VAR=value` prefix in commands like `FOO=bar cmd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvAssignment {
    pub name: String,
    pub value: Word,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirection {
    /// `>`, `>>`, `2>`, etc.
    Out { fd: Option<u32>, target: Word, append: bool },
    /// `<`
    In { fd: Option<u32>, source: Word },
    /// `<<DELIM` or `<<-DELIM`
    HereDoc {
        fd: Option<u32>,
        delimiter: String,
        body: String,
        strip_tabs: bool,
        /// Quoted delimiters suppress expansion.
        quoted: bool,
    },
    /// `<<<`
    HereString { fd: Option<u32>, content: Word },
    /// `&>` or `&>>`
    Both { append: bool, target: Word },
    /// `n>&m`, `n<&m`, `n>&-`, `n<&-`
    Duplicate {
        source: u32,
        target: super::token::DupTarget,
        /// True for output dup (`>&`), false for input dup (`<&`).
        output: bool,
    },
}

impl Redirection {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    pub env: Vec<EnvAssignment>,
    pub name: Word,
    pub args: Vec<Word>,
    pub redirections: Vec<Redirection>,
    pub span: Span,
}

/// A word composed of literal, variable, and substitution parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub parts: Vec<WordPart>,
    pub span: Span,
}

/// Preserved from parsing to control glob suppression and variable expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuoteStyle {
    /// Subject to glob expansion and word splitting.
    #[default]
    Unquoted,
    /// No expansion, no glob.
    Single,
    /// Variable/command expansion enabled, no glob.
    Double,
}

/// Unparsed at the token level, parsed into AST by the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubstitutionBody {
    Unparsed(String),
    Parsed(Box<CommandList>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordPart {
    Literal { value: String, quoted: QuoteStyle },
    Variable { name: String, modifier: Option<String> },
    CommandSubstitution { body: SubstitutionBody, backtick: bool },
}

impl WordPart {
    pub fn literal(value: impl Into<String>) -> Self {
        WordPart::Literal { value: value.into(), quoted: QuoteStyle::Unquoted }
    }

    pub fn single_quoted(value: impl Into<String>) -> Self {
        WordPart::Literal { value: value.into(), quoted: QuoteStyle::Single }
    }

    pub fn double_quoted(value: impl Into<String>) -> Self {
        WordPart::Literal { value: value.into(), quoted: QuoteStyle::Double }
    }
}

// ── CLI argument parsing ─────────────────────────────────────────────────

/// A parsed CLI argument from a command's argument list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliArg<'a> {
    /// `-v` or `-abc` (multiple flags bundled).
    ShortFlag(&'a Word),
    /// `--verbose` (no value).
    LongFlag(&'a Word),
    /// `--key=value` (inline value).
    LongOptionInline {
        word: &'a Word,
        key: &'a str,
        value: &'a str,
    },
    /// `--key value` or `--key val1 val2` (separate value(s)).
    LongOptionSeparate {
        key_word: &'a Word,
        key: &'a str,
        value_words: Vec<&'a Word>,
    },
    Positional(&'a Word),
}

impl<'a> CliArg<'a> {
    pub fn is_flag(&self) -> bool {
        matches!(self, CliArg::ShortFlag(_) | CliArg::LongFlag(_))
    }

    pub fn is_option(&self) -> bool {
        matches!(self, CliArg::LongOptionInline { .. } | CliArg::LongOptionSeparate { .. })
    }

    pub fn is_positional(&self) -> bool {
        matches!(self, CliArg::Positional(_))
    }

    pub fn option_key(&self) -> Option<&str> {
        match self {
            CliArg::LongOptionInline { key, .. } => Some(key),
            CliArg::LongOptionSeparate { key, .. } => Some(key),
            _ => None,
        }
    }
}

impl SimpleCommand {
    /// Parse arguments into CLI categories using standard conventions.
    ///
    /// `options_with_values` specifies which long options consume the next argument
    /// as their value. Without this, we can't distinguish `--model haiku` (option
    /// with value) from `--print prompt` (flag + positional).
    ///
    /// `multi_value_options` specifies which long options consume all following
    /// non-flag arguments as values (e.g., `--disallowed-tools A B C`).
    pub fn parse_cli_args(
        &self,
        options_with_values: &[&str],
        multi_value_options: &[&str],
    ) -> Vec<CliArg<'_>> {
        let mut result = Vec::new();
        let mut i = 0;

        while i < self.args.len() {
            let word = &self.args[i];

            let first_literal = word.parts.first().and_then(|p| match p {
                WordPart::Literal { value, .. } => Some(value.as_str()),
                _ => None,
            });

            match first_literal {
                Some(s) if s.starts_with("--") => {
                    if let Some(eq_pos) = s.find('=') {
                        let key = &s[2..eq_pos];
                        let value = &s[eq_pos + 1..];
                        result.push(CliArg::LongOptionInline { word, key, value });
                    } else {
                        let key = &s[2..];
                        let is_multi = multi_value_options.contains(&key);
                        let takes_value = is_multi || options_with_values.contains(&key);
                        let has_next = i + 1 < self.args.len();

                        if takes_value && has_next {
                            if is_multi {
                                let mut value_words = Vec::new();
                                while i + 1 < self.args.len() {
                                    let next = &self.args[i + 1];
                                    let is_flag = next.parts.first().is_some_and(|p| match p {
                                        WordPart::Literal { value, .. } => value.starts_with('-'),
                                        _ => false,
                                    });
                                    if is_flag {
                                        break;
                                    }
                                    value_words.push(next);
                                    i += 1;
                                }
                                result.push(CliArg::LongOptionSeparate {
                                    key_word: word,
                                    key,
                                    value_words,
                                });
                            } else {
                                let value_word = &self.args[i + 1];
                                result.push(CliArg::LongOptionSeparate {
                                    key_word: word,
                                    key,
                                    value_words: vec![value_word],
                                });
                                i += 1;
                            }
                        } else {
                            result.push(CliArg::LongFlag(word));
                        }
                    }
                }
                Some(s) if s.starts_with('-') && s.len() > 1 => {
                    result.push(CliArg::ShortFlag(word));
                }
                _ => {
                    result.push(CliArg::Positional(word));
                }
            }

            i += 1;
        }

        result
    }

    /// Matches both `--name` and `--name=...` forms.
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

    /// See [`parse_cli_args`] for parameter semantics.
    pub fn positional_args(
        &self,
        options_with_values: &[&str],
        multi_value_options: &[&str],
    ) -> Vec<&Word> {
        self.parse_cli_args(options_with_values, multi_value_options)
            .into_iter()
            .filter_map(|arg| match arg {
                CliArg::Positional(w) => Some(w),
                _ => None,
            })
            .collect()
    }
}

// ── Utility methods ──────────────────────────────────────────────────────

impl CommandList {
    /// Convenience wrapper around [`Parser::parse`].
    ///
    /// [`Parser::parse`]: super::Parser::parse
    pub fn parse(input: &str) -> Result<Self, super::parse_error::ParseError> {
        super::parser::Parser::parse(input)
    }

    /// Count the total number of simple commands in the AST.
    ///
    /// Includes commands in jobs, subshells, brace groups,
    /// and command substitutions.
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
    /// first appear. Includes variables in double-quoted strings,
    /// unquoted expansions, and command substitutions.
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

    /// Check if the AST contains any `$(...)` or backtick substitutions,
    /// including nested inside other substitutions.
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
    /// Returns 0 for a flat command list. Only counts subshells `(...)`
    /// and brace groups `{...}`, not command substitutions.
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

// ── Visitor ──────────────────────────────────────────────────────────────

/// Visitor pattern for walking the shell AST.
///
/// Each `visit_*` method has a corresponding `walk_*` method. The `visit_*`
/// method is called at a node, and can call `walk_*` to descend into children.
/// To stop traversal at a node, simply don't call `walk_*`.
pub trait AstVisitor {
    fn visit_command_list(&mut self, cmd_list: &CommandList) {
        self.walk_command_list(cmd_list);
    }

    fn visit_and_or_list(&mut self, and_or: &AndOrList) {
        self.walk_and_or_list(and_or);
    }

    fn visit_command_item(&mut self, item: &CommandItem) {
        self.walk_command_item(item);
    }

    fn visit_command(&mut self, command: &Command) {
        self.walk_command(command);
    }

    fn visit_simple_command(&mut self, cmd: &SimpleCommand) {
        self.walk_simple_command(cmd);
    }

    fn visit_job(&mut self, job: &Job) {
        self.walk_job(job);
    }

    fn visit_subshell(&mut self, subshell: &Subshell) {
        self.walk_subshell(subshell);
    }

    fn visit_brace_group(&mut self, group: &BraceGroup) {
        self.walk_brace_group(group);
    }

    fn visit_word(&mut self, word: &Word) {
        self.walk_word(word);
    }

    fn visit_word_part(&mut self, part: &WordPart) {
        self.walk_word_part(part);
    }

    fn visit_env_assignment(&mut self, assignment: &EnvAssignment) {
        self.walk_env_assignment(assignment);
    }

    fn visit_redirection(&mut self, redir: &Redirection) {
        self.walk_redirection(redir);
    }

    fn walk_command_list(&mut self, cmd_list: &CommandList) {
        for and_or in &cmd_list.commands {
            self.visit_and_or_list(and_or);
        }
    }

    fn walk_and_or_list(&mut self, and_or: &AndOrList) {
        self.visit_command_item(&and_or.first);
        for (_, item) in &and_or.rest {
            self.visit_command_item(item);
        }
    }

    fn walk_command_item(&mut self, item: &CommandItem) {
        self.visit_command(&item.command);
    }

    fn walk_command(&mut self, command: &Command) {
        match command {
            Command::Simple(cmd) => self.visit_simple_command(cmd),
            Command::Job(p) => self.visit_job(p),
            Command::Subshell(s) => self.visit_subshell(s),
            Command::BraceGroup(b) => self.visit_brace_group(b),
        }
    }

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

    fn walk_env_assignment(&mut self, assignment: &EnvAssignment) {
        self.visit_word(&assignment.value);
    }

    fn walk_redirection(&mut self, redir: &Redirection) {
        match redir {
            Redirection::Out { target, .. } => self.visit_word(target),
            Redirection::In { source, .. } => self.visit_word(source),
            Redirection::HereString { content, .. } => self.visit_word(content),
            Redirection::Both { target, .. } => self.visit_word(target),
            // HereDoc body is a pre-parsed string, no words to visit
            Redirection::HereDoc { .. } => {}
            Redirection::Duplicate { .. } => {}
        }
    }

    fn walk_job(&mut self, job: &Job) {
        for cmd in &job.commands {
            self.visit_simple_command(cmd);
        }
    }

    fn walk_subshell(&mut self, subshell: &Subshell) {
        self.visit_command_list(&subshell.body);
        for redir in &subshell.redirections {
            self.visit_redirection(redir);
        }
    }

    fn walk_brace_group(&mut self, group: &BraceGroup) {
        self.visit_command_list(&group.body);
        for redir in &group.redirections {
            self.visit_redirection(redir);
        }
    }

    fn walk_word(&mut self, word: &Word) {
        for part in &word.parts {
            self.visit_word_part(part);
        }
    }

    fn walk_word_part(&mut self, part: &WordPart) {
        if let WordPart::CommandSubstitution { body: SubstitutionBody::Parsed(body), .. } = part {
            self.visit_command_list(body);
        }
    }
}

#[cfg(test)]
#[path = "ast_tests.rs"]
mod tests;
