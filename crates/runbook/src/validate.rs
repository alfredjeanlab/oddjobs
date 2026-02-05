// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Validation helpers for runbook parsing

use crate::parser::ParseError;
use oj_shell as shell;
use std::collections::{HashMap, HashSet};

/// Agent commands recognized at parse time.
///
/// Commands not in this list will produce a parse error, preventing typos and
/// ensuring only supported agent adapters are referenced.
const SUPPORTED_AGENT_COMMANDS: &[&str] = &["claude", "claudeless"];

/// Claude/claudeless CLI options that take a single value argument.
///
/// These options consume the next argument as their value, so we need to know
/// them to correctly identify positional arguments in the command.
/// Options with `=` syntax (e.g., `--model=haiku`) are handled automatically.
///
/// Options that accept multiple space-separated values belong in
/// [`CLAUDE_MULTI_VALUE_OPTIONS`] instead.
const CLAUDE_OPTIONS_WITH_VALUES: &[&str] = &[
    // claude options
    "agent",
    "agents",
    "append-system-prompt",
    "debug",
    "debug-file",
    "fallback-model",
    "file",
    "from-pr",
    "input-format",
    "json-schema",
    "max-budget-usd",
    "mcp-config",
    "model",
    "output-format",
    "permission-mode",
    "plugin-dir",
    "resume",
    "session-id",
    "setting-sources",
    "settings",
    "system-prompt",
    // claudeless options
    "scenario",
    "tool-mode",
];

/// Claude/claudeless CLI options that accept multiple space-separated values.
///
/// These options consume all following non-flag arguments as values.
/// For example, `--disallowed-tools ExitPlanMode AskUserQuestion` treats
/// both `ExitPlanMode` and `AskUserQuestion` as values for `--disallowed-tools`.
const CLAUDE_MULTI_VALUE_OPTIONS: &[&str] = &[
    "add-dir",
    "allowed-tools",
    "allowedTools",
    "betas",
    "disallowed-tools",
    "disallowedTools",
    "tools",
];

/// Namespaces that are only available in job context, not in command.run.
///
/// Each entry maps from the invalid namespace to a suggestion for the user.
const JOB_ONLY_NAMESPACES: &[(&str, &str)] = &[
    ("var.", "use ${args.<name>} to reference command arguments"),
    (
        "input.",
        "use ${args.<name>} to reference command arguments",
    ),
    ("local.", "${local.*} is only available in job steps"),
    ("step.", "${step.*} is only available in job steps"),
];

/// Validate that a command.run shell directive does not use job-only
/// template namespaces like `${var.*}` (which should be `${args.*}`).
pub(crate) fn validate_command_template_refs(
    command: &str,
    location: &str,
) -> Result<(), ParseError> {
    for cap in crate::template::VAR_PATTERN.captures_iter(command) {
        let var_name = &cap[1];
        for &(prefix, hint) in JOB_ONLY_NAMESPACES {
            if var_name.starts_with(prefix) {
                return Err(ParseError::InvalidFormat {
                    location: location.to_string(),
                    message: format!(
                        "template reference ${{{}}} is not available in command.run; {}",
                        var_name, hint,
                    ),
                });
            }
        }
        // Also reject ${workspace.*} (dotted), since command context only has ${workspace}
        if var_name.starts_with("workspace.") {
            return Err(ParseError::InvalidFormat {
                location: location.to_string(),
                message: format!(
                    "template reference ${{{}}} is not available in command.run; \
                     commands have ${{workspace}} (the execution path), not ${{workspace.*}}",
                    var_name,
                ),
            });
        }
    }
    Ok(())
}

/// Validate a shell command string, returning an error with context on failure.
///
/// Template variables like `${name}` are replaced with placeholder strings before
/// validation to avoid conflicts with shell brace group syntax.
pub(crate) fn validate_shell_command(command: &str, location: &str) -> Result<(), ParseError> {
    // Replace template variables with placeholders to avoid brace conflicts
    let normalized = crate::template::VAR_PATTERN.replace_all(command, "_VAR_");
    let source_text = normalized.to_string();
    let ast = shell::Parser::parse(&normalized).map_err(|inner| ParseError::ShellError {
        location: location.to_string(),
        inner: Box::new(inner),
        source_text: source_text.clone(),
    })?;
    if let Err(errors) = shell::validate(&ast) {
        if let Some(inner) = errors.into_iter().next() {
            return Err(ParseError::ShellValidation {
                location: location.to_string(),
                inner: Box::new(inner),
                source_text,
            });
        }
    }
    Ok(())
}

/// Validate a duration string like "30s", "5m", "1h", "0s".
pub(crate) fn validate_duration_str(s: &str) -> Result<(), String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration string".to_string());
    }

    let (num_str, suffix) = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| (&s[..i], &s[i..]))
        .unwrap_or((s, ""));

    let _num: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number in duration: {}", s))?;

    match suffix.trim() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" | "ms" | "millis" | "millisecond"
        | "milliseconds" | "m" | "min" | "mins" | "minute" | "minutes" | "h" | "hr" | "hrs"
        | "hour" | "hours" | "d" | "day" | "days" => Ok(()),
        other => Err(format!("unknown duration suffix: {}", other)),
    }
}

/// Validate that an agent's run command uses a recognized agent command.
///
/// Parses the shell AST and extracts the first command name (taking basename
/// to handle absolute paths like `/usr/local/bin/claude`). If the command name
/// can't be statically determined (e.g. variable-only name), validation is skipped.
///
/// If `has_prompt` is true, positional arguments are rejected since the system
/// will append the prompt automatically.
pub(crate) fn validate_agent_command(
    command: &str,
    location: &str,
    has_prompt: bool,
) -> Result<(), ParseError> {
    let normalized = crate::template::VAR_PATTERN.replace_all(command, "_VAR_");
    let source_text = normalized.to_string();
    let ast = shell::Parser::parse(&normalized).map_err(|inner| ParseError::ShellError {
        location: location.to_string(),
        inner: Box::new(inner),
        source_text,
    })?;

    // Walk to the first SimpleCommand in the AST
    let first_cmd = ast.commands.first().map(|and_or| &and_or.first.command);

    let simple = match first_cmd {
        Some(shell::Command::Simple(cmd)) => cmd,
        Some(shell::Command::Job(p)) => match p.commands.first() {
            Some(cmd) => cmd,
            None => return Ok(()),
        },
        _ => return Ok(()),
    };

    // Extract the command name if the first part is a literal
    let name = match simple.name.parts.first() {
        Some(shell::WordPart::Literal { value, .. }) => value.as_str(),
        _ => return Ok(()), // Variable or substitution — skip validation
    };

    // Take basename to handle absolute paths
    let basename = name.rsplit('/').next().unwrap_or(name);

    if !SUPPORTED_AGENT_COMMANDS.contains(&basename) {
        return Err(ParseError::InvalidFormat {
            location: location.to_string(),
            message: format!(
                "unrecognized agent command '{}'; must be one of: {}",
                basename,
                SUPPORTED_AGENT_COMMANDS.join(", "),
            ),
        });
    }

    // Reject --session-id flag (the system adds it automatically)
    if simple.has_long_option("session-id") {
        return Err(ParseError::InvalidFormat {
            location: location.to_string(),
            message: format!(
                "{} command must not include '--session-id' (added automatically)",
                basename,
            ),
        });
    }

    // Reject positional arguments when prompt field is configured (system appends it)
    if has_prompt
        && !simple
            .positional_args(CLAUDE_OPTIONS_WITH_VALUES, CLAUDE_MULTI_VALUE_OPTIONS)
            .is_empty()
    {
        return Err(ParseError::InvalidFormat {
            location: location.to_string(),
            message: format!(
                "{} command must not include positional arguments when prompt is configured (use prompt field or inline, not both)",
                basename,
            ),
        });
    }

    Ok(())
}

/// Valid template namespaces for job context.
///
/// Template references must use one of these known prefixes to prevent typos
/// like `${vra.name}` from silently failing at runtime.
const VALID_TEMPLATE_NAMESPACES: &[&str] = &[
    "var",
    "args",
    "item",
    "local",
    "workspace",
    "invoke",
    "prompt",
    "step",
];

/// Validate that template references use recognized namespaces.
///
/// Catches typos like `${vra.name}` (should be `${var.name}`) at parse time.
/// Single-segment references (no dot) are allowed as they may reference
/// environment variables or simple placeholders.
pub(crate) fn validate_template_namespaces(
    template: &str,
    location: &str,
) -> Result<(), ParseError> {
    for cap in crate::template::VAR_PATTERN.captures_iter(template) {
        let var_name = &cap[1];
        // Only validate namespaced refs (containing a dot)
        if let Some(dot_pos) = var_name.find('.') {
            let namespace = &var_name[..dot_pos];
            if !VALID_TEMPLATE_NAMESPACES.contains(&namespace) {
                return Err(ParseError::InvalidFormat {
                    location: location.to_string(),
                    message: format!(
                        "unrecognized template namespace '{}' in ${{{}}}; valid namespaces: {}",
                        namespace,
                        var_name,
                        VALID_TEMPLATE_NAMESPACES.join(", "),
                    ),
                });
            }
        }
    }
    Ok(())
}

/// Sort and join names from a HashSet for deterministic error messages.
pub(crate) fn sorted_names(names: &HashSet<&str>) -> String {
    let mut v: Vec<&str> = names.iter().copied().collect();
    v.sort();
    v.join(", ")
}

/// Sort and join keys from a HashMap for deterministic error messages.
pub(crate) fn sorted_keys<V>(map: &HashMap<String, V>) -> String {
    let mut v: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
    v.sort();
    v.join(", ")
}
