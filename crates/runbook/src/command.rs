// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Command definitions

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Errors that can occur during argument spec parsing
#[derive(Debug, Error)]
pub enum ArgSpecError {
    #[error("invalid argument syntax: {0}")]
    InvalidSyntax(String),
    #[error("variadic must be last: {0}")]
    VariadicNotLast(String),
    #[error("optional positional cannot precede required: {0}")]
    OptionalBeforeRequired(String),
    #[error("duplicate argument name: {0}")]
    DuplicateName(String),
}

/// Errors that can occur during argument validation
#[derive(Debug, Error)]
pub enum ArgValidationError {
    #[error("missing required argument: <{0}>")]
    MissingPositional(String),
    #[error("missing required option: --{0}")]
    MissingOption(String),
    #[error("missing required argument: <{0}...>")]
    MissingVariadic(String),
    #[error("unexpected positional argument: {0}")]
    UnexpectedPositional(String),
    #[error("unknown option: --{0}")]
    UnknownOption(String),
}

/// A positional argument definition
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArgDef {
    pub name: String,
    pub required: bool,
}

/// A flag definition (boolean switch)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlagDef {
    pub name: String,
    pub short: Option<char>,
}

/// An option definition (flag with value)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionDef {
    pub name: String,
    pub short: Option<char>,
    pub required: bool,
}

/// A variadic argument definition (accepts multiple values)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VariadicDef {
    pub name: String,
    pub required: bool,
}

/// Argument specification for a command
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ArgSpec {
    /// Positional arguments in order
    pub positional: Vec<ArgDef>,
    /// Boolean flags
    pub flags: Vec<FlagDef>,
    /// Options with values
    pub options: Vec<OptionDef>,
    /// Variadic argument (must be last)
    pub variadic: Option<VariadicDef>,
}

/// Parse an argument specification string
///
/// Supports:
/// - `<name>` - required positional
/// - `[name]` - optional positional
/// - `<files...>` - required variadic
/// - `[files...]` - optional variadic
/// - `--flag` - boolean flag
/// - `-f/--flag` - flag with short alias
/// - `--opt <val>` - required option with value
/// - `[--opt <val>]` - optional option with value
/// - `[-o/--opt <val>]` - optional option with short alias
pub fn parse_arg_spec(spec: &str) -> Result<ArgSpec, ArgSpecError> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Ok(ArgSpec::default());
    }

    let mut result = ArgSpec::default();
    let mut seen_optional_positional = false;
    let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut chars = spec.chars().peekable();
    let mut current_token = String::new();

    // Helper to check for duplicate names
    let mut check_name = |name: &str| -> Result<(), ArgSpecError> {
        if !names.insert(name.to_string()) {
            return Err(ArgSpecError::DuplicateName(name.to_string()));
        }
        Ok(())
    };

    while let Some(c) = chars.next() {
        match c {
            '<' => {
                // Required positional or variadic
                current_token.clear();
                for nc in chars.by_ref() {
                    if nc == '>' {
                        break;
                    }
                    current_token.push(nc);
                }

                let name = current_token.trim();
                if name.ends_with("...") {
                    // Variadic
                    let var_name = name.trim_end_matches("...");
                    check_name(var_name)?;
                    if result.variadic.is_some() {
                        return Err(ArgSpecError::VariadicNotLast(var_name.to_string()));
                    }
                    result.variadic = Some(VariadicDef {
                        name: var_name.to_string(),
                        required: true,
                    });
                } else {
                    // Required positional
                    if result.variadic.is_some() {
                        return Err(ArgSpecError::VariadicNotLast(name.to_string()));
                    }
                    if seen_optional_positional {
                        return Err(ArgSpecError::OptionalBeforeRequired(name.to_string()));
                    }
                    check_name(name)?;
                    result.positional.push(ArgDef {
                        name: name.to_string(),
                        required: true,
                    });
                }
            }
            '[' => {
                // Optional positional, variadic, or option
                current_token.clear();
                let mut bracket_depth = 1;
                for nc in chars.by_ref() {
                    if nc == '[' {
                        bracket_depth += 1;
                    } else if nc == ']' {
                        bracket_depth -= 1;
                        if bracket_depth == 0 {
                            break;
                        }
                    }
                    current_token.push(nc);
                }

                let content = current_token.trim();
                if content.starts_with('-') {
                    // Optional flag or option: [--flag] or [--opt <val>] or [-o/--opt <val>]
                    parse_flag_or_option(content, false, &mut result, &mut check_name)?;
                } else if content.ends_with("...") {
                    // Optional variadic
                    let var_name = content.trim_end_matches("...");
                    check_name(var_name)?;
                    if result.variadic.is_some() {
                        return Err(ArgSpecError::VariadicNotLast(var_name.to_string()));
                    }
                    result.variadic = Some(VariadicDef {
                        name: var_name.to_string(),
                        required: false,
                    });
                } else {
                    // Optional positional
                    if result.variadic.is_some() {
                        return Err(ArgSpecError::VariadicNotLast(content.to_string()));
                    }
                    check_name(content)?;
                    seen_optional_positional = true;
                    result.positional.push(ArgDef {
                        name: content.to_string(),
                        required: false,
                    });
                }
            }
            '-' => {
                // Required flag or option at top level: --flag or --opt <val>
                // Put back the dash and read the whole token
                current_token.clear();
                current_token.push('-');
                while let Some(&nc) = chars.peek() {
                    if nc.is_whitespace() {
                        break;
                    }
                    if let Some(nc) = chars.next() {
                        current_token.push(nc);
                    }
                }

                // Check if there's a following <val>
                skip_whitespace(&mut chars);
                if chars.peek() == Some(&'<') {
                    chars.next(); // consume '<'
                    let mut val_name = String::new();
                    for nc in chars.by_ref() {
                        if nc == '>' {
                            break;
                        }
                        val_name.push(nc);
                    }
                    // Required option
                    let opt_content = format!("{} <{}>", current_token, val_name);
                    parse_flag_or_option(&opt_content, true, &mut result, &mut check_name)?;
                } else {
                    // Required flag
                    parse_flag_or_option(&current_token, true, &mut result, &mut check_name)?;
                }
            }
            c if c.is_whitespace() => {
                // Skip whitespace
                continue;
            }
            _ => {
                return Err(ArgSpecError::InvalidSyntax(format!(
                    "unexpected character: {}",
                    c
                )));
            }
        }
    }

    Ok(result)
}

fn skip_whitespace(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else {
            break;
        }
    }
}

fn parse_flag_or_option<F>(
    content: &str,
    required: bool,
    result: &mut ArgSpec,
    check_name: &mut F,
) -> Result<(), ArgSpecError>
where
    F: FnMut(&str) -> Result<(), ArgSpecError>,
{
    let content = content.trim();

    // Check if this is an option (has <val>)
    if let Some(val_start) = content.find('<') {
        // Option with value
        let flag_part = content[..val_start].trim();
        let (short, name) = parse_flag_names(flag_part)?;
        check_name(&name)?;
        result.options.push(OptionDef {
            name,
            short,
            required,
        });
    } else {
        // Boolean flag
        let (short, name) = parse_flag_names(content)?;
        check_name(&name)?;
        result.flags.push(FlagDef { name, short });
    }

    Ok(())
}

fn parse_flag_names(flag_part: &str) -> Result<(Option<char>, String), ArgSpecError> {
    let flag_part = flag_part.trim();

    // Handle -f/--flag syntax
    if flag_part.contains('/') {
        let parts: Vec<&str> = flag_part.split('/').collect();
        if parts.len() != 2 {
            return Err(ArgSpecError::InvalidSyntax(format!(
                "invalid flag syntax: {}",
                flag_part
            )));
        }
        let short_part = parts[0].trim();
        let long_part = parts[1].trim();

        let short = if short_part.starts_with('-') && !short_part.starts_with("--") {
            short_part.chars().nth(1)
        } else {
            return Err(ArgSpecError::InvalidSyntax(format!(
                "invalid short flag: {}",
                short_part
            )));
        };

        let name = if let Some(stripped) = long_part.strip_prefix("--") {
            stripped.to_string()
        } else {
            return Err(ArgSpecError::InvalidSyntax(format!(
                "invalid long flag: {}",
                long_part
            )));
        };

        Ok((short, name))
    } else if let Some(stripped) = flag_part.strip_prefix("--") {
        // Long flag only
        Ok((None, stripped.to_string()))
    } else if flag_part.starts_with('-') {
        // Short flag only - use the char as the name
        let c = flag_part
            .chars()
            .nth(1)
            .ok_or_else(|| ArgSpecError::InvalidSyntax("empty flag".to_string()))?;
        Ok((Some(c), c.to_string()))
    } else {
        Err(ArgSpecError::InvalidSyntax(format!(
            "flag must start with -: {}",
            flag_part
        )))
    }
}

impl ArgSpec {
    /// Format as a usage string, e.g. "<name> <instructions> [--base <branch>]"
    pub fn usage_line(&self) -> String {
        let mut parts = Vec::new();
        for arg in &self.positional {
            if arg.required {
                parts.push(format!("<{}>", arg.name));
            } else {
                parts.push(format!("[{}]", arg.name));
            }
        }
        if let Some(v) = &self.variadic {
            if v.required {
                parts.push(format!("<{}...>", v.name));
            } else {
                parts.push(format!("[{}...]", v.name));
            }
        }
        for opt in &self.options {
            if opt.required {
                parts.push(format!("--{} <{}>", opt.name, opt.name));
            } else {
                parts.push(format!("[--{} <{}>]", opt.name, opt.name));
            }
        }
        for flag in &self.flags {
            parts.push(format!("[--{}]", flag.name));
        }
        parts.join(" ")
    }

    /// Get all positional argument names in order
    pub fn positional_names(&self) -> Vec<&str> {
        self.positional.iter().map(|a| a.name.as_str()).collect()
    }

    /// Split raw CLI arguments into positional and named arguments.
    ///
    /// This allows flags and options defined in the ArgSpec to appear anywhere
    /// relative to positional args (standard CLI behavior).
    /// e.g., `kanban --new kanban` produces positional=["kanban"] named={"new": "kanban"}
    ///
    /// Unknown flags (not in the ArgSpec) are kept as positional args.
    pub fn split_raw_args(&self, raw: &[String]) -> (Vec<String>, HashMap<String, String>) {
        let mut positional = Vec::new();
        let mut named = HashMap::new();
        let mut i = 0;

        while i < raw.len() {
            let arg = &raw[i];

            if arg == "--" {
                // Everything after -- is positional
                positional.extend_from_slice(&raw[i + 1..]);
                break;
            }

            if let Some(stripped) = arg.strip_prefix("--") {
                if let Some(opt) = self.options.iter().find(|o| o.name == stripped) {
                    // Known option: consume next arg as value
                    if i + 1 < raw.len() {
                        named.insert(opt.name.clone(), raw[i + 1].clone());
                        i += 2;
                    } else {
                        positional.push(arg.clone());
                        i += 1;
                    }
                } else if self.flags.iter().any(|f| f.name == stripped) {
                    named.insert(stripped.to_string(), "true".to_string());
                    i += 1;
                } else {
                    // Unknown flag, keep as positional (will fail validation if unexpected)
                    positional.push(arg.clone());
                    i += 1;
                }
            } else if arg.starts_with('-') && arg.len() == 2 {
                let Some(c) = arg.chars().nth(1) else {
                    positional.push(arg.clone());
                    i += 1;
                    continue;
                };
                if let Some(opt) = self.options.iter().find(|o| o.short == Some(c)) {
                    if i + 1 < raw.len() {
                        named.insert(opt.name.clone(), raw[i + 1].clone());
                        i += 2;
                    } else {
                        positional.push(arg.clone());
                        i += 1;
                    }
                } else if let Some(flag) = self.flags.iter().find(|f| f.short == Some(c)) {
                    named.insert(flag.name.clone(), "true".to_string());
                    i += 1;
                } else {
                    positional.push(arg.clone());
                    i += 1;
                }
            } else {
                positional.push(arg.clone());
                i += 1;
            }
        }

        (positional, named)
    }
}

// Custom deserializer: accepts either a string (TOML format) or a struct (JSON round-trip)
impl<'de> Deserialize<'de> for ArgSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de;

        struct ArgSpecVisitor;

        impl<'de> de::Visitor<'de> for ArgSpecVisitor {
            type Value = ArgSpec;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an arg spec string or struct")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<ArgSpec, E> {
                parse_arg_spec(value).map_err(de::Error::custom)
            }

            fn visit_map<M: de::MapAccess<'de>>(self, map: M) -> Result<ArgSpec, M::Error> {
                // Struct form from JSON round-trip
                #[derive(Deserialize)]
                struct ArgSpecFields {
                    #[serde(default)]
                    positional: Vec<ArgDef>,
                    #[serde(default)]
                    flags: Vec<FlagDef>,
                    #[serde(default)]
                    options: Vec<OptionDef>,
                    #[serde(default)]
                    variadic: Option<VariadicDef>,
                }
                let fields =
                    ArgSpecFields::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(ArgSpec {
                    positional: fields.positional,
                    flags: fields.flags,
                    options: fields.options,
                    variadic: fields.variadic,
                })
            }
        }

        deserializer.deserialize_any(ArgSpecVisitor)
    }
}

/// A command definition from the runbook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDef {
    /// Command name (e.g., "build", "test")
    #[serde(default)]
    pub name: String,
    /// Short description for help text (e.g., "Run a build pipeline")
    #[serde(default)]
    pub description: Option<String>,
    /// Argument specification
    #[serde(default)]
    pub args: ArgSpec,
    /// Default values for arguments
    #[serde(default)]
    pub defaults: HashMap<String, String>,
    /// What to run when the command is invoked
    pub run: RunDirective,
}

/// What a command or step should execute
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RunDirective {
    /// Shell command string: `run = "echo hello"`
    Shell(String),
    /// Pipeline reference: `run = { pipeline = "build" }`
    Pipeline { pipeline: String },
    /// Agent reference: `run = { agent = "planning" }`
    Agent { agent: String },
}

impl RunDirective {
    /// Check if this is a shell command
    pub fn is_shell(&self) -> bool {
        matches!(self, RunDirective::Shell(_))
    }

    /// Check if this is a pipeline reference
    pub fn is_pipeline(&self) -> bool {
        matches!(self, RunDirective::Pipeline { .. })
    }

    /// Check if this is an agent reference
    pub fn is_agent(&self) -> bool {
        matches!(self, RunDirective::Agent { .. })
    }

    /// Get the shell command if this is a shell directive
    pub fn shell_command(&self) -> Option<&str> {
        match self {
            RunDirective::Shell(cmd) => Some(cmd),
            _ => None,
        }
    }

    /// Get the pipeline name if this is a pipeline directive
    pub fn pipeline_name(&self) -> Option<&str> {
        match self {
            RunDirective::Pipeline { pipeline } => Some(pipeline),
            _ => None,
        }
    }

    /// Get the agent name if this is an agent directive
    pub fn agent_name(&self) -> Option<&str> {
        match self {
            RunDirective::Agent { agent } => Some(agent),
            _ => None,
        }
    }
}

impl CommandDef {
    /// Validate that all required arguments are provided and no unknown arguments are given
    pub fn validate_args(
        &self,
        positional: &[String],
        named: &HashMap<String, String>,
    ) -> Result<(), ArgValidationError> {
        // Build set of known names (positional, options, flags, variadic)
        let known_names: HashSet<&str> = self
            .args
            .positional
            .iter()
            .map(|a| a.name.as_str())
            .chain(self.args.options.iter().map(|o| o.name.as_str()))
            .chain(self.args.flags.iter().map(|f| f.name.as_str()))
            .chain(self.args.variadic.iter().map(|v| v.name.as_str()))
            .collect();

        // Check all named args are known (catches typos early)
        for key in named.keys() {
            if !known_names.contains(key.as_str()) {
                return Err(ArgValidationError::UnknownOption(key.clone()));
            }
        }

        // Check required positional arguments
        for (i, arg_def) in self.args.positional.iter().enumerate() {
            if arg_def.required {
                let has_value = positional.get(i).is_some()
                    || named.contains_key(&arg_def.name)
                    || self.defaults.contains_key(&arg_def.name);
                if !has_value {
                    return Err(ArgValidationError::MissingPositional(arg_def.name.clone()));
                }
            }
        }

        // Check required options
        for opt_def in &self.args.options {
            if opt_def.required {
                let has_value =
                    named.contains_key(&opt_def.name) || self.defaults.contains_key(&opt_def.name);
                if !has_value {
                    return Err(ArgValidationError::MissingOption(opt_def.name.clone()));
                }
            }
        }

        // Check required variadic
        if let Some(variadic) = &self.args.variadic {
            if variadic.required {
                let start_idx = self.args.positional.len();
                let has_values = positional.len() > start_idx
                    || named.contains_key(&variadic.name)
                    || self.defaults.contains_key(&variadic.name);
                if !has_values {
                    return Err(ArgValidationError::MissingVariadic(variadic.name.clone()));
                }
            }
        }

        // Check for unexpected positional arguments (only if no variadic defined)
        if self.args.variadic.is_none() {
            let max_positional = self.args.positional.len();
            if positional.len() > max_positional {
                let extra = &positional[max_positional];
                return Err(ArgValidationError::UnexpectedPositional(extra.clone()));
            }
        }

        Ok(())
    }

    /// Parse arguments from CLI input and merge with defaults
    pub fn parse_args(
        &self,
        positional: &[String],
        named: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        let mut result = self.defaults.clone();

        // Map positional args to their names
        for (i, arg_def) in self.args.positional.iter().enumerate() {
            if let Some(value) = positional.get(i) {
                result.insert(arg_def.name.clone(), value.clone());
            }
        }

        // Handle variadic args
        if let Some(variadic) = &self.args.variadic {
            let start_idx = self.args.positional.len();
            if positional.len() > start_idx {
                let values: Vec<&str> =
                    positional[start_idx..].iter().map(|s| s.as_str()).collect();
                result.insert(variadic.name.clone(), values.join(" "));
            }
        }

        // Named args (options) override
        for (key, value) in named {
            result.insert(key.clone(), value.clone());
        }

        result
    }
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
