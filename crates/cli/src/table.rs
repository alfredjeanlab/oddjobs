// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Shared table renderer for CLI list views.
//!
//! Provides consistent column alignment, color application, and truncation
//! across all `oj * list` commands.

use std::collections::HashSet;
use std::io::Write;

use crate::color;

/// Column text alignment.
pub enum Align {
    Left,
    Right,
}

/// How a cell's text is styled after padding.
pub enum CellStyle {
    /// No color applied.
    Plain,
    /// Apply [`color::muted()`].
    Muted,
    /// Apply [`color::status()`] — auto-detects green/yellow/red.
    Status,
}

/// A column definition in a [`Table`].
pub struct Column {
    pub name: &'static str,
    pub align: Align,
    pub style: CellStyle,
    /// Minimum width (defaults to header text length).
    pub min_width: Option<usize>,
    /// Maximum width (`None` = unlimited). Values exceeding this are truncated.
    pub max_width: Option<usize>,
}

impl Column {
    /// Left-aligned, plain style.
    pub fn left(name: &'static str) -> Self {
        Self {
            name,
            align: Align::Left,
            style: CellStyle::Plain,
            min_width: None,
            max_width: None,
        }
    }

    /// Right-aligned, plain style.
    pub fn right(name: &'static str) -> Self {
        Self {
            name,
            align: Align::Right,
            style: CellStyle::Plain,
            min_width: None,
            max_width: None,
        }
    }

    /// Left-aligned, muted style.
    pub fn muted(name: &'static str) -> Self {
        Self {
            name,
            align: Align::Left,
            style: CellStyle::Muted,
            min_width: None,
            max_width: None,
        }
    }

    /// Left-aligned, status style.
    pub fn status(name: &'static str) -> Self {
        Self {
            name,
            align: Align::Left,
            style: CellStyle::Status,
            min_width: None,
            max_width: None,
        }
    }

    /// Set maximum width (values exceeding this are truncated).
    pub fn with_max(mut self, max: usize) -> Self {
        self.max_width = Some(max);
        self
    }
}

/// A tabular renderer that auto-computes column widths from data.
pub struct Table {
    columns: Vec<Column>,
    rows: Vec<Vec<String>>,
    colorize: bool,
}

/// Column separator: double space.
const SEP: &str = "  ";

impl Table {
    pub fn new(columns: Vec<Column>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            colorize: color::should_colorize(),
        }
    }

    /// Create a table that never emits color codes.
    #[cfg(test)]
    pub fn plain(columns: Vec<Column>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            colorize: false,
        }
    }

    /// Create a table that always emits color codes.
    #[cfg(test)]
    pub fn colored(columns: Vec<Column>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
            colorize: true,
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    /// Render the full table (header + rows) to the given writer.
    ///
    /// Column widths are auto-computed from data. The last column is never
    /// padded. Color is applied **after** padding so ANSI escapes don't
    /// corrupt width calculations.
    pub fn render(&self, out: &mut impl Write) {
        if self.rows.is_empty() {
            return;
        }

        let widths = self.compute_widths();

        let colorize = self.colorize;

        // Header row
        let header_cells: Vec<String> = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let is_last = i == self.columns.len() - 1;
                let w = widths[i];
                let padded = if is_last && matches!(col.align, Align::Left) {
                    col.name.to_string()
                } else {
                    pad(col.name, w, &col.align)
                };
                if colorize {
                    color::apply_header(&padded)
                } else {
                    padded
                }
            })
            .collect();
        let _ = writeln!(out, "{}", header_cells.join(SEP));

        // Data rows
        for row in &self.rows {
            let cells: Vec<String> = self
                .columns
                .iter()
                .enumerate()
                .map(|(i, col)| {
                    let is_last = i == self.columns.len() - 1;
                    let w = widths[i];
                    let raw = row.get(i).map(|s| s.as_str()).unwrap_or("");
                    let truncated = truncate(raw, col.max_width);
                    let padded = if is_last && matches!(col.align, Align::Left) {
                        truncated.to_string()
                    } else {
                        pad(truncated, w, &col.align)
                    };
                    stylize(&padded, &col.style, colorize)
                })
                .collect();
            let _ = writeln!(out, "{}", cells.join(SEP));
        }
    }

    /// Compute the display width for each column.
    fn compute_widths(&self) -> Vec<usize> {
        self.columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let min = col.min_width.unwrap_or(col.name.len());
                let max_data = self
                    .rows
                    .iter()
                    .map(|row| {
                        let raw = row.get(i).map(|s| s.len()).unwrap_or(0);
                        // If max_width is set, truncated value is at most max_width
                        match col.max_width {
                            Some(mw) => raw.min(mw),
                            None => raw,
                        }
                    })
                    .max()
                    .unwrap_or(0);
                min.max(max_data)
            })
            .collect()
    }
}

/// Determines if a PROJECT column should be shown based on namespace diversity.
///
/// Returns `true` when items span multiple namespaces OR any namespace is non-empty.
pub fn should_show_project<'a>(namespaces: impl Iterator<Item = &'a str>) -> bool {
    let set: HashSet<&str> = namespaces.collect();
    set.len() > 1 || set.iter().any(|n| !n.is_empty())
}

/// Formats a namespace for display in a PROJECT column.
pub fn project_cell(namespace: &str) -> String {
    if namespace.is_empty() {
        "(no project)".to_string()
    } else {
        namespace.to_string()
    }
}

/// Pad a string to `width` using the given alignment.
fn pad(text: &str, width: usize, align: &Align) -> String {
    match align {
        Align::Left => format!("{:<width$}", text),
        Align::Right => format!("{:>width$}", text),
    }
}

/// Truncate a string to at most `max` characters (if set).
fn truncate(s: &str, max: Option<usize>) -> &str {
    match max {
        Some(m) if s.len() > m => &s[..m],
        _ => s,
    }
}

/// Apply a [`CellStyle`] to already-padded text.
fn stylize(text: &str, style: &CellStyle, colorize: bool) -> String {
    if !colorize {
        return text.to_string();
    }
    match style {
        CellStyle::Plain => text.to_string(),
        CellStyle::Muted => color::apply_muted(text),
        CellStyle::Status => color::apply_status(text),
    }
}

#[cfg(test)]
#[path = "table_tests.rs"]
mod tests;
