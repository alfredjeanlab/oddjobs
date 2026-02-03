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
    // KEEP UNTIL: list/status command coloring
    #[allow(dead_code)]
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
        .header(Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(codes::HEADER)))))
        .literal(Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(codes::LITERAL)))))
        .placeholder(Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(codes::CONTEXT)))))
}

// KEEP UNTIL: list/status command coloring
#[allow(dead_code)]
fn fg256(code: u8) -> String {
    format!("\x1b[38;5;{code}m")
}

// KEEP UNTIL: list/status command coloring
#[allow(dead_code)]
const RESET: &str = "\x1b[0m";

/// Format text with the header color (steel blue).
// KEEP UNTIL: list/status command coloring
#[allow(dead_code)]
pub fn header(text: &str) -> String {
    if should_colorize() {
        format!("{}{}{}", fg256(codes::HEADER), text, RESET)
    } else {
        text.to_string()
    }
}

/// Format text with the literal color (light grey).
// KEEP UNTIL: list/status command coloring
#[allow(dead_code)]
pub fn literal(text: &str) -> String {
    if should_colorize() {
        format!("{}{}{}", fg256(codes::LITERAL), text, RESET)
    } else {
        text.to_string()
    }
}

/// Format text with the context color (medium grey).
// KEEP UNTIL: list/status command coloring
#[allow(dead_code)]
pub fn context(text: &str) -> String {
    if should_colorize() {
        format!("{}{}{}", fg256(codes::CONTEXT), text, RESET)
    } else {
        text.to_string()
    }
}

/// Format text with the muted color (darker grey).
// KEEP UNTIL: list/status command coloring
#[allow(dead_code)]
pub fn muted(text: &str) -> String {
    if should_colorize() {
        format!("{}{}{}", fg256(codes::MUTED), text, RESET)
    } else {
        text.to_string()
    }
}

#[cfg(test)]
#[path = "color_tests.rs"]
mod tests;
