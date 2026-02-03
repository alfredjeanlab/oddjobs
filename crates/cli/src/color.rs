// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use clap::builder::styling::{Ansi256Color, Color, Style, Styles};
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
}

/// Determine if color output should be enabled.
///
/// Priority: `NO_COLOR=1` disables → `COLOR=1` forces → TTY check.
pub fn should_colorize() -> bool {
    if std::env::var("NO_COLOR").is_ok_and(|v| v == "1") {
        return false;
    }
    if std::env::var("COLOR").is_ok_and(|v| v == "1") {
        return true;
    }
    std::io::stdout().is_terminal()
}

/// Build clap `Styles` using the project palette.
pub fn styles() -> Styles {
    if !should_colorize() {
        return Styles::plain();
    }
    Styles::styled()
        .usage(Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(codes::HEADER)))))
        .header(Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(codes::HEADER)))))
        .literal(Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(codes::LITERAL)))))
        .placeholder(Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(codes::CONTEXT)))))
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
        format!("{}{}{}", fg256(codes::HEADER), text, RESET)
    } else {
        text.to_string()
    }
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
        format!("{}{}{}", fg256(codes::MUTED), text, RESET)
    } else {
        text.to_string()
    }
}

/// Colorize a status string based on its semantic meaning.
///
/// - Green: completed, done, running, started (healthy active states)
/// - Yellow: waiting, escalated, pending, idle, orphaned, stopping, stopped
/// - Red: failed, cancelled, dead, gone, error
/// - Default (no color): unknown states
pub fn status(text: &str) -> String {
    if !should_colorize() {
        return text.to_string();
    }
    let lower = text.to_lowercase();
    let code = match lower.as_str() {
        "completed" | "done" | "running" | "started" => "\x1b[32m",
        "waiting" | "escalated" | "pending" | "idle" | "orphaned" | "stopping" | "stopped" => {
            "\x1b[33m"
        }
        "failed" | "cancelled" | "dead" | "gone" | "error" => "\x1b[31m",
        _ => return text.to_string(),
    };
    format!("{code}{text}{RESET}")
}

#[cfg(test)]
#[path = "color_tests.rs"]
mod tests;
