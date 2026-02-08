// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::io::IsTerminal;

pub mod codes {
    /// Section headers: pastel cyan / steel blue (matches wok & quench)
    pub const HEADER: u8 = 74;
    /// Commands and literals: light grey
    pub const LITERAL: u8 = 250;
    /// Descriptions and context: medium grey
    pub const CONTEXT: u8 = 245;
    /// Muted / secondary text: darker grey
    pub const MUTED: u8 = 240;

    /// Pre-formatted ANSI escape sequences for use in tests
    #[cfg(test)]
    pub const HEADER_START: &str = "\x1b[38;5;74m";
    #[cfg(test)]
    pub const LITERAL_START: &str = "\x1b[38;5;250m";
    #[cfg(test)]
    pub const CONTEXT_START: &str = "\x1b[38;5;245m";
    #[cfg(test)]
    pub const RESET: &str = "\x1b[0m";
}

/// Determine if color output should be enabled.
///
/// Priority: `NO_COLOR=1` disables → `COLOR=1` forces → TTY check.
pub fn should_colorize() -> bool {
    if crate::env::no_color() {
        return false;
    }
    if crate::env::force_color() {
        return true;
    }
    std::io::stdout().is_terminal()
}

fn fg256(code: u8) -> String {
    format!("\x1b[38;5;{code}m")
}

const RESET: &str = "\x1b[0m";

/// Builder for clap-style colored help text.
///
/// Matches the palette from [`styles()`] so custom help blocks
/// look identical to clap's built-in output.
pub struct HelpPrinter {
    buf: String,
    colorize: bool,
}

impl HelpPrinter {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            colorize: should_colorize(),
        }
    }

    /// Create a printer that never emits color codes (for tests).
    #[cfg(test)]
    pub fn uncolored() -> Self {
        Self {
            buf: String::new(),
            colorize: false,
        }
    }

    /// "Usage: <rest>" — header-colored label, plain rest.
    pub fn usage(&mut self, rest: &str) {
        use std::fmt::Write;
        if self.colorize {
            let _ = writeln!(self.buf, "{}Usage:{} {rest}", fg256(codes::HEADER), RESET,);
        } else {
            let _ = writeln!(self.buf, "Usage: {rest}");
        }
    }

    /// Section header (e.g. "Available Commands:").
    pub fn header(&mut self, label: &str) {
        use std::fmt::Write;
        if self.colorize {
            let _ = writeln!(self.buf, "{}{label}{}", fg256(codes::HEADER), RESET);
        } else {
            let _ = writeln!(self.buf, "{label}");
        }
    }

    /// Two-column entry: literal-colored name padded to `width`, optional description.
    pub fn entry(&mut self, name: &str, width: usize, desc: Option<&str>) {
        use std::fmt::Write;
        if self.colorize {
            if let Some(desc) = desc {
                let _ = writeln!(
                    self.buf,
                    "  {}{name:<width$}{} {desc}",
                    fg256(codes::LITERAL),
                    RESET,
                );
            } else {
                let _ = writeln!(self.buf, "  {}{name}{}", fg256(codes::LITERAL), RESET);
            }
        } else if let Some(desc) = desc {
            let _ = writeln!(self.buf, "  {name:<width$} {desc}");
        } else {
            let _ = writeln!(self.buf, "  {name}");
        }
    }

    /// Hint / footer line in context color.
    pub fn hint(&mut self, text: &str) {
        use std::fmt::Write;
        if self.colorize {
            let _ = writeln!(self.buf, "{}{text}{}", fg256(codes::CONTEXT), RESET);
        } else {
            let _ = writeln!(self.buf, "{text}");
        }
    }

    /// Plain text line (no color).
    pub fn plain(&mut self, text: &str) {
        use std::fmt::Write;
        let _ = writeln!(self.buf, "{text}");
    }

    /// Blank line.
    pub fn blank(&mut self) {
        self.buf.push('\n');
    }

    /// Consume the printer and return the formatted string.
    pub fn finish(self) -> String {
        self.buf
    }
}

/// Format text with the header color (steel blue).
pub fn header(text: &str) -> String {
    if should_colorize() {
        apply_header(text)
    } else {
        text.to_string()
    }
}

/// Apply header color unconditionally (caller decides whether to use this).
pub(crate) fn apply_header(text: &str) -> String {
    format!("{}{}{}", fg256(codes::HEADER), text, RESET)
}

/// Format text with the context color (medium grey).
pub fn context(text: &str) -> String {
    if should_colorize() {
        format!("{}{}{}", fg256(codes::CONTEXT), text, RESET)
    } else {
        text.to_string()
    }
}

/// Format text with the muted color (darker grey).
pub fn muted(text: &str) -> String {
    if should_colorize() {
        apply_muted(text)
    } else {
        text.to_string()
    }
}

/// Apply muted color unconditionally (caller decides whether to use this).
pub(crate) fn apply_muted(text: &str) -> String {
    format!("{}{}{}", fg256(codes::MUTED), text, RESET)
}

/// Apply green (ANSI 32) to text, respecting color settings.
pub fn green(text: &str) -> String {
    if !should_colorize() {
        return text.to_string();
    }
    format!("\x1b[32m{text}{RESET}")
}

/// Apply yellow (ANSI 33) to text, respecting color settings.
pub fn yellow(text: &str) -> String {
    if !should_colorize() {
        return text.to_string();
    }
    format!("\x1b[33m{text}{RESET}")
}

/// Colorize a status string based on its semantic meaning.
///
/// - Green: completed, done, running, started, ready (healthy active states)
/// - Yellow: waiting, escalated, pending, idle, orphaned, stopping, stopped,
///   creating, cleaning
/// - Red: failed, cancelled, dead, gone, error
/// - Default (no color): unknown states
///
/// Uses first-word matching so compound statuses like "failed: reason" and
/// "waiting (decision-id)" are colored correctly.
pub fn status(text: &str) -> String {
    if !should_colorize() {
        return text.to_string();
    }
    apply_status(text)
}

/// Apply status color unconditionally (caller decides whether to use this).
pub(crate) fn apply_status(text: &str) -> String {
    let lower = text.trim_start().to_lowercase();
    let first_word = lower
        .split(|c: char| !c.is_alphabetic())
        .next()
        .unwrap_or("");
    let code = match first_word {
        "completed" | "done" | "running" | "started" | "ready" | "on" => "\x1b[32m",
        "waiting" | "escalated" | "pending" | "idle" | "orphaned" | "suspended" | "stopping"
        | "stopped" | "creating" | "cleaning" | "full" | "off" => "\x1b[33m",
        "failed" | "cancelled" | "dead" | "gone" | "error" => "\x1b[31m",
        _ => return text.to_string(),
    };
    format!("{code}{text}{RESET}")
}

#[cfg(test)]
#[path = "color_tests.rs"]
mod tests;
