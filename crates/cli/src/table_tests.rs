// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use serial_test::serial;

fn render_to_string(table: &Table) -> String {
    let mut buf = Vec::new();
    table.render(&mut buf);
    String::from_utf8(buf).unwrap()
}

#[test]
fn empty_table_prints_nothing() {
    let table = Table::new(vec![Column::left("NAME"), Column::left("STATUS")]);
    let out = render_to_string(&table);
    assert_eq!(out, "");
}

#[test]
fn single_row_single_column() {
    let mut table = Table::new(vec![Column::left("NAME")]);
    table.row(vec!["hello".into()]);
    let out = render_to_string(&table);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "NAME");
    assert_eq!(lines[1], "hello");
}

#[test]
fn multi_column_left_alignment() {
    let mut table = Table::new(vec![Column::left("NAME"), Column::left("KIND")]);
    table.row(vec!["alpha".into(), "build".into()]);
    table.row(vec!["b".into(), "fix".into()]);
    let out = render_to_string(&table);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 3);
    // Header: "NAME" padded to 5 (width of "alpha"), "KIND" not padded (last col)
    assert_eq!(lines[0], "NAME   KIND");
    assert_eq!(lines[1], "alpha  build");
    assert_eq!(lines[2], "b      fix");
}

#[test]
fn right_alignment() {
    let mut table = Table::new(vec![Column::left("NAME"), Column::right("COUNT")]);
    table.row(vec!["alpha".into(), "5".into()]);
    table.row(vec!["beta".into(), "123".into()]);
    let out = render_to_string(&table);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 3);
    // COUNT is the last column, so no padding applied
    assert_eq!(lines[0], "NAME   COUNT");
    assert_eq!(lines[1], "alpha      5");
    assert_eq!(lines[2], "beta     123");
}

#[test]
fn column_width_adapts_to_widest_cell() {
    let mut table = Table::new(vec![Column::left("ID"), Column::left("STATUS")]);
    table.row(vec!["a".into(), "ok".into()]);
    table.row(vec!["longvalue".into(), "error".into()]);
    let out = render_to_string(&table);
    let lines: Vec<&str> = out.lines().collect();

    // ID column should be padded to "longvalue" width (9)
    assert_eq!(lines[0], "ID         STATUS");
    assert_eq!(lines[1], "a          ok");
    assert_eq!(lines[2], "longvalue  error");
}

#[test]
fn max_width_truncates_long_values() {
    let mut table = Table::new(vec![Column::left("ID").with_max(4), Column::left("NAME")]);
    table.row(vec!["abcdef".into(), "test".into()]);
    let out = render_to_string(&table);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines[1], "abcd  test");
}

#[test]
fn min_width_enforces_minimum() {
    let mut table = Table::new(vec![
        {
            let mut c = Column::left("X");
            c.min_width = Some(10);
            c
        },
        Column::left("Y"),
    ]);
    table.row(vec!["a".into(), "b".into()]);
    let out = render_to_string(&table);
    let lines: Vec<&str> = out.lines().collect();

    // X column should be at least 10 wide
    assert_eq!(lines[0], "X           Y");
    assert_eq!(lines[1], "a           b");
}

#[test]
fn last_column_no_trailing_padding() {
    let mut table = Table::new(vec![Column::left("A"), Column::left("B")]);
    table.row(vec!["short".into(), "x".into()]);
    table.row(vec!["s".into(), "longvalue".into()]);
    let out = render_to_string(&table);
    let lines: Vec<&str> = out.lines().collect();

    // Last column should not be padded — "x" should not have trailing spaces
    assert_eq!(lines[1], "short  x");
    assert_eq!(lines[2], "s      longvalue");
}

#[test]
fn double_space_column_separator() {
    let mut table = Table::new(vec![
        Column::left("A"),
        Column::left("B"),
        Column::left("C"),
    ]);
    table.row(vec!["1".into(), "2".into(), "3".into()]);
    let out = render_to_string(&table);
    let lines: Vec<&str> = out.lines().collect();

    // Columns separated by exactly "  " (double space)
    assert_eq!(lines[1], "1  2  3");
}

#[test]
#[serial]
fn muted_style_applies_ansi_when_color_enabled() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let mut table = Table::new(vec![Column::muted("ID")]);
    table.row(vec!["abc".into()]);
    let out = render_to_string(&table);

    // Should contain ANSI escape for muted (code 240)
    assert!(
        out.contains("\x1b[38;5;240m"),
        "should have muted ANSI code in: {:?}",
        out
    );
    assert!(out.contains("\x1b[0m"), "should have reset code");

    std::env::remove_var("COLOR");
}

#[test]
#[serial]
fn status_style_applies_ansi_when_color_enabled() {
    std::env::set_var("COLOR", "1");
    std::env::remove_var("NO_COLOR");

    let mut table = Table::new(vec![Column::status("STATUS")]);
    table.row(vec!["Running".into()]);
    let out = render_to_string(&table);

    // "Running" → green
    assert!(
        out.contains("\x1b[32m"),
        "should have green ANSI code in: {:?}",
        out
    );

    std::env::remove_var("COLOR");
}

#[test]
#[serial]
fn no_ansi_when_no_color() {
    std::env::set_var("NO_COLOR", "1");

    let mut table = Table::new(vec![Column::muted("ID"), Column::status("STATUS")]);
    table.row(vec!["abc".into(), "Running".into()]);
    let out = render_to_string(&table);

    assert!(
        !out.contains("\x1b["),
        "should have no ANSI codes in: {:?}",
        out
    );

    std::env::remove_var("NO_COLOR");
}

#[test]
fn right_aligned_non_last_column() {
    let mut table = Table::new(vec![
        Column::left("NAME"),
        Column::right("COUNT"),
        Column::left("STATUS"),
    ]);
    table.row(vec!["alpha".into(), "5".into(), "ok".into()]);
    table.row(vec!["beta".into(), "123".into(), "err".into()]);
    let out = render_to_string(&table);
    let lines: Vec<&str> = out.lines().collect();

    // COUNT is right-aligned in the middle
    assert_eq!(lines[0], "NAME   COUNT  STATUS");
    assert_eq!(lines[1], "alpha      5  ok");
    assert_eq!(lines[2], "beta     123  err");
}
