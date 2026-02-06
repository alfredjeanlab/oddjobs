// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Sectioned help output with post-hoc colorization.
//!
//! This module provides help output formatting that:
//! - Organizes subcommands into Actions, Resources, and System sections
//! - Uses `Styles::plain()` so clap generates uncolored output
//! - Applies colors post-hoc via `colorize_help()` for full control
//!
//! The pattern follows wok's approach: plain text generation → post-hoc colorization.

use std::io::Write;

use clap::builder::styling::Styles;
use clap::Command;

use crate::color;

/// Generate clap Styles for help output.
///
/// Returns `Styles::plain()` because we apply colors manually after
/// capturing clap's output.
pub fn styles() -> Styles {
    Styles::plain()
}

/// Main help template.
/// Uses `{before-help}` for sectioned commands and `{after-help}` for optional trailing content.
/// Colors are applied later by `colorize_help()` since clap strips ANSI codes from template values.
pub fn template() -> String {
    "{about-with-newline}\n{usage-heading} {usage}\n\n{before-help}Options:\n{options}{after-help}"
        .to_string()
}

/// Commands list shown before options in main help.
///
/// Returns plain text organized into Actions, Resources, and System sections.
/// Colors are applied later by `format_help()` because clap's `Styles::plain()`
/// strips ANSI codes from template values.
pub fn commands() -> String {
    "\
Actions:
  run         Run a command from the runbook
  cancel      Cancel one or more running jobs
  resume      Resume an escalated job
  status      Show overview of active work across all projects
  show        Show details of a job, agent, session, or queue
  peek        Peek at the active tmux session
  attach      Attach to a tmux session

Resources:
  job    Job management
  agent       Agent management
  session     Session management
  workspace   Workspace management
  queue       Queue management
  worker      Worker management
  cron        Cron management
  decision    Decision management
  project     Project management
  runbook     Runbook management

System:
  env         Environment variable management
  logs        View logs for a job or agent
  emit        Emit events to the daemon
  daemon      Daemon management"
        .to_string()
}

/// Optional trailing section (examples, quickstart, etc.).
/// Returns empty for now; infrastructure is in place for future use.
pub fn after_help() -> String {
    String::new()
}

/// Format help output for a command with post-hoc colorization.
///
/// Captures clap's plain help output and applies the oj color palette.
/// Takes ownership so we can set `Styles::plain()` — subcommands cloned
/// via `find_subcommand` don't inherit the parent's styles, so without
/// this they'd use clap's default colored styles and bypass `colorize_help`.
pub fn format_help(cmd: Command) -> String {
    let mut cmd = cmd.styles(styles());
    let mut buf = Vec::new();
    match cmd.write_help(&mut buf) {
        Ok(()) => {}
        Err(_) => unreachable!("write_help to Vec<u8> is infallible"),
    }
    let raw_help = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(_) => unreachable!("clap help output is always valid UTF-8"),
    };

    let output = if color::should_colorize() {
        colorize_help(&raw_help)
    } else {
        raw_help
    };

    if output.ends_with('\n') {
        output
    } else {
        format!("{}\n", output)
    }
}

/// Print formatted help to stdout.
pub fn print_help(cmd: Command) {
    let help = format_help(cmd);
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(help.as_bytes());
    let _ = stdout.flush();
}

/// Print pre-formatted help text to stdout with post-hoc colorization.
///
/// Used for help text generated outside of clap (e.g., runbook commands).
pub fn print_help_text(raw_help: &str) {
    let output = if color::should_colorize() {
        colorize_help(raw_help)
    } else {
        raw_help.to_string()
    };
    let output = if output.ends_with('\n') {
        output
    } else {
        format!("{}\n", output)
    };
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(output.as_bytes());
    let _ = stdout.flush();
}

/// Apply oj palette to plain help text.
///
/// Recognizes:
/// - Section headers (lines ending with `:` without `  `) → HEADER color
/// - `Usage:` prefix → HEADER color
/// - Command lines (2-space indent + name + 2+ spaces + desc) → LITERAL for name
/// - Option lines (indented, starting with `-`) → LITERAL for flags, CONTEXT for placeholders
pub fn colorize_help(text: &str) -> String {
    let mut result = Vec::new();

    for line in text.lines() {
        // Skip lines that already have ANSI escape codes
        if line.contains("\x1b[") {
            result.push(line.to_string());
            continue;
        }

        // Section headers (lines ending with `:` without double-space)
        if line.ends_with(':') && !line.contains("  ") {
            result.push(apply_header(line));
            continue;
        }

        // Usage line
        if line.starts_with("Usage:") {
            let parts: Vec<&str> = line.splitn(2, ' ').collect();
            if parts.len() == 2 {
                result.push(format!("{} {}", apply_header(parts[0]), parts[1]));
            } else {
                result.push(line.to_string());
            }
            continue;
        }

        // Command list line (2-space indent, not an option)
        if let Some(colored) = colorize_command_line(line) {
            result.push(colored);
            continue;
        }

        // Option line (indented, starts with -)
        if let Some(colored) = colorize_option_line(line) {
            result.push(colored);
            continue;
        }

        // Keep other lines as-is
        result.push(line.to_string());
    }

    result.join("\n")
}

/// Apply header color unconditionally.
fn apply_header(text: &str) -> String {
    format!("{}{}{}", fg256(color::codes::HEADER), text, RESET)
}

/// Apply literal color unconditionally.
fn apply_literal(text: &str) -> String {
    format!("{}{}{}", fg256(color::codes::LITERAL), text, RESET)
}

/// Apply context color unconditionally.
fn apply_context(text: &str) -> String {
    format!("{}{}{}", fg256(color::codes::CONTEXT), text, RESET)
}

fn fg256(code: u8) -> String {
    format!("\x1b[38;5;{code}m")
}

const RESET: &str = "\x1b[0m";

/// Colorize a command list line (2-space indent + name + description).
fn colorize_command_line(line: &str) -> Option<String> {
    if !line.starts_with("  ") || line.starts_with("   ") {
        return None;
    }

    let trimmed = line.trim_start();

    // Option lines start with - → handled by colorize_option_line
    if trimmed.starts_with('-') {
        return None;
    }

    // Find where the command name ends (before 2+ spaces)
    let cmd_end = trimmed.find("  ").unwrap_or(trimmed.len());
    if cmd_end == 0 {
        return None;
    }

    let cmd = &trimmed[..cmd_end];
    let rest = &trimmed[cmd_end..];

    Some(format!("  {}{}", apply_literal(cmd), rest))
}

/// Colorize an option line (indented, starts with `-` or has `--`).
///
/// Parses option lines like:
///   `-C <DIR>                 Description`
///   `    --project <PROJECT>  Description`
///   `-o, --output <OUTPUT>    Description [default: text]`
///   `-v, --version            Print version`
///   `-h, --help               Print help`
fn colorize_option_line(line: &str) -> Option<String> {
    // Must be indented
    if !line.starts_with("  ") {
        return None;
    }

    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];

    // Must start with a dash (option flag)
    if !trimmed.starts_with('-') {
        return None;
    }

    // Find the description start (2+ consecutive spaces after the flags/value section)
    // We need to find where flags+value end and description begins
    let desc_start = find_description_start(trimmed);

    let (flags_part, desc_part) = if let Some(pos) = desc_start {
        (&trimmed[..pos], &trimmed[pos..])
    } else {
        (trimmed, "")
    };

    // Colorize the flags portion
    let colored_flags = colorize_flags(flags_part);

    // Colorize description, highlighting [default: ...] and [possible values: ...]
    let colored_desc = colorize_option_description(desc_part);

    Some(format!("{}{}{}", indent, colored_flags, colored_desc))
}

/// Find where the description starts (after 2+ spaces following the flags section).
fn find_description_start(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut in_spaces = false;
    let mut space_start = 0;
    // Skip initial flag characters before looking for description gap
    let mut past_flags = false;

    while i < bytes.len() {
        if bytes[i] == b' ' {
            if !in_spaces {
                in_spaces = true;
                space_start = i;
            }
        } else {
            if in_spaces && past_flags && i - space_start >= 2 {
                return Some(space_start);
            }
            in_spaces = false;
            past_flags = true;
        }
        i += 1;
    }

    None
}

/// Colorize the flags portion of an option line.
/// E.g. "-o, --output <OUTPUT>" → literal("-o") + ", " + literal("--output") + " " + context("<OUTPUT>")
fn colorize_flags(flags: &str) -> String {
    let mut result = String::with_capacity(flags.len() + 64);
    let mut i = 0;
    let bytes = flags.as_bytes();

    while i < bytes.len() {
        if bytes[i] == b'-' {
            // Start of a flag (short or long)
            let start = i;
            while i < bytes.len() && bytes[i] != b' ' && bytes[i] != b',' {
                i += 1;
            }
            result.push_str(&apply_literal(&flags[start..i]));
        } else if bytes[i] == b'<' {
            // Placeholder like <DIR>
            let start = i;
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1; // include '>'
            }
            result.push_str(&apply_context(&flags[start..i]));
        } else {
            // Commas, spaces, etc.
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

/// Colorize option description, highlighting bracketed metadata as context.
fn colorize_option_description(desc: &str) -> String {
    if desc.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(desc.len() + 64);
    let mut i = 0;
    let bytes = desc.as_bytes();

    while i < bytes.len() {
        if bytes[i] == b'[' {
            let start = i;
            let mut depth = 1;
            i += 1;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'[' {
                    depth += 1;
                } else if bytes[i] == b']' {
                    depth -= 1;
                }
                i += 1;
            }
            let bracketed = &desc[start..i];
            result.push_str(&apply_context(bracketed));
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

#[cfg(test)]
#[path = "help_tests.rs"]
mod tests;
