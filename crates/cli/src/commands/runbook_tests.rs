// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use clap::Parser;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: RunbookCommand,
}

#[test]
fn parse_list_subcommand() {
    let cli = Cli::try_parse_from(["test", "list"]).unwrap();
    assert!(matches!(cli.command, RunbookCommand::List {}));
}

#[test]
fn parse_search_no_query() {
    let cli = Cli::try_parse_from(["test", "search"]).unwrap();
    assert!(matches!(cli.command, RunbookCommand::Search { query: None }));
}

#[test]
fn parse_search_with_query() {
    let cli = Cli::try_parse_from(["test", "search", "wok"]).unwrap();
    assert!(matches!(cli.command, RunbookCommand::Search { query: Some(q) } if q == "wok"));
}

#[test]
fn parse_info_subcommand() {
    let cli = Cli::try_parse_from(["test", "info", "oj/wok"]).unwrap();
    assert!(matches!(cli.command, RunbookCommand::Info { path } if path == "oj/wok"));
}

#[test]
fn parse_add_subcommand() {
    let cli = Cli::try_parse_from(["test", "add", "./my-lib"]).unwrap();
    assert!(matches!(
        cli.command,
        RunbookCommand::Add { path, name: None, project: false } if path == "./my-lib"
    ));
}

#[test]
fn parse_add_with_name() {
    let cli = Cli::try_parse_from(["test", "add", "./path", "--name", "mylib"]).unwrap();
    assert!(matches!(
        cli.command,
        RunbookCommand::Add { path, name: Some(n), project: false } if path == "./path" && n == "mylib"
    ));
}

#[test]
fn parse_add_project_flag() {
    let cli = Cli::try_parse_from(["test", "add", "./path", "--project"]).unwrap();
    assert!(matches!(cli.command, RunbookCommand::Add { project: true, .. }));
}

#[test]
fn search_filters_by_query() {
    let libraries = oj_runbook::available_libraries(&[]);

    let q = "wok";
    let q_lower = q.to_lowercase();
    let filtered: Vec<_> = libraries
        .into_iter()
        .filter(|lib| {
            lib.source.to_lowercase().contains(&q_lower)
                || lib.description.to_lowercase().contains(&q_lower)
        })
        .collect();

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].source, "oj/wok");
}
