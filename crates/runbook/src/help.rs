// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Help text formatting for runbook commands

use std::fmt::Write;

use crate::command::CommandDef;
use crate::find::FileComment;

/// Format a clap-style `--help` page for a command definition.
pub fn format_command_help(
    cmd: &CommandDef,
    command_name: &str,
    comment: Option<&FileComment>,
) -> String {
    let mut out = String::new();

    // 1. Short description (from description field or comment)
    if let Some(desc) = cmd
        .description
        .as_deref()
        .or(comment.map(|c| c.short.as_str()))
    {
        out.push_str(desc);
        out.push_str("\n\n");
    }

    // 2. Usage line
    let usage = cmd.args.usage_line();
    if usage.is_empty() {
        let _ = writeln!(out, "Usage: oj run {command_name}");
    } else {
        let _ = writeln!(out, "Usage: oj run {command_name} {usage}");
    }

    // 3. Arguments section (positional + variadic)
    if !cmd.args.positional.is_empty() || cmd.args.variadic.is_some() {
        out.push_str("\nArguments:\n");
        for arg in &cmd.args.positional {
            let label = if arg.required {
                format!("  <{}>", arg.name)
            } else {
                format!("  [{}]", arg.name)
            };
            if let Some(default) = cmd.defaults.get(&arg.name) {
                if !default.is_empty() {
                    let _ = writeln!(out, "{label:<24} [default: {default}]");
                    continue;
                }
            }
            let _ = writeln!(out, "{label}");
        }
        if let Some(v) = &cmd.args.variadic {
            let label = if v.required {
                format!("  <{}...>", v.name)
            } else {
                format!("  [{}...]", v.name)
            };
            let _ = writeln!(out, "{label}");
        }
    }

    // 4. Options section (options + flags)
    if !cmd.args.options.is_empty() || !cmd.args.flags.is_empty() {
        out.push_str("\nOptions:\n");
        for opt in &cmd.args.options {
            let short = opt.short.map(|c| format!("-{c}, ")).unwrap_or_default();
            let label = format!("  {short}--{} <{}>", opt.name, opt.name);
            if let Some(default) = cmd.defaults.get(&opt.name) {
                if !default.is_empty() {
                    let _ = writeln!(out, "{label:<24} [default: {default}]");
                } else {
                    let _ = writeln!(out, "{label}");
                }
            } else {
                let _ = writeln!(out, "{label}");
            }
        }
        for flag in &cmd.args.flags {
            let short = flag.short.map(|c| format!("-{c}, ")).unwrap_or_default();
            let label = format!("  {short}--{}", flag.name);
            let _ = writeln!(out, "{label}");
        }
    }

    // 5. Description section (long comment, with Usage: → Examples:)
    if let Some(comment) = comment {
        if !comment.long.is_empty() {
            let rewritten = comment
                .long
                .lines()
                .map(|line| {
                    if line.trim_start().starts_with("Usage:") {
                        line.replacen("Usage:", "Examples:", 1)
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            out.push_str("\nDescription:\n");
            for line in rewritten.lines() {
                let _ = writeln!(out, "  {line}");
            }
        }
    }

    out
}

#[cfg(test)]
#[path = "help_tests.rs"]
mod tests;
