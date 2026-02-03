// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::{Command, Parser};

fn parse_simple(input: &str) -> crate::SimpleCommand {
    let ast = Parser::parse(input).unwrap();
    match ast.commands[0].first.command.clone() {
        Command::Simple(c) => c,
        _ => panic!("expected simple command"),
    }
}

#[test]
fn single_value_option_consumes_one_arg() {
    let cmd = parse_simple("claude --model haiku pos1");
    let args = cmd.parse_cli_args(&["model"], &[]);
    assert!(args[0].is_option());
    assert_eq!(args[0].option_key(), Some("model"));
    assert!(args[1].is_positional());
}

#[test]
fn multi_value_option_consumes_all_following_non_flag_args() {
    let cmd = parse_simple("claude --disallowed-tools ExitPlanMode AskUserQuestion EnterPlanMode");
    let args = cmd.parse_cli_args(&[], &["disallowed-tools"]);
    assert_eq!(args.len(), 1);
    assert!(args[0].is_option());
    assert_eq!(args[0].option_key(), Some("disallowed-tools"));
}

#[test]
fn multi_value_option_stops_at_next_flag() {
    let cmd = parse_simple("claude --disallowed-tools ExitPlanMode AskUserQuestion --verbose");
    let args = cmd.parse_cli_args(&[], &["disallowed-tools"]);
    assert_eq!(args.len(), 2);
    assert!(args[0].is_option());
    assert_eq!(args[0].option_key(), Some("disallowed-tools"));
    assert!(args[1].is_flag()); // --verbose
}

#[test]
fn multi_value_option_stops_at_next_option() {
    let cmd = parse_simple("claude --disallowed-tools ExitPlanMode --model haiku");
    let args = cmd.parse_cli_args(&["model"], &["disallowed-tools"]);
    assert_eq!(args.len(), 2);
    assert_eq!(args[0].option_key(), Some("disallowed-tools"));
    assert_eq!(args[1].option_key(), Some("model"));
}

#[test]
fn multi_value_no_positionals_with_space_separated_values() {
    let cmd = parse_simple("claude --disallowed-tools ExitPlanMode AskUserQuestion");
    let positionals = cmd.positional_args(&[], &["disallowed-tools"]);
    assert!(positionals.is_empty());
}

#[test]
fn multi_value_single_value_still_works() {
    let cmd = parse_simple("claude --disallowed-tools ExitPlanMode");
    let args = cmd.parse_cli_args(&[], &["disallowed-tools"]);
    assert_eq!(args.len(), 1);
    assert!(args[0].is_option());
}

#[test]
fn multi_value_with_other_options_interspersed() {
    let cmd = parse_simple(
        "claude --model opus --disallowed-tools ExitPlanMode AskUserQuestion --verbose",
    );
    let args = cmd.parse_cli_args(&["model"], &["disallowed-tools"]);
    assert_eq!(args.len(), 3);
    assert_eq!(args[0].option_key(), Some("model"));
    assert_eq!(args[1].option_key(), Some("disallowed-tools"));
    assert!(args[2].is_flag()); // --verbose
}

#[test]
fn positional_args_with_multi_value_options() {
    // Positional arg BEFORE multi-value option should still be detected
    let cmd = parse_simple("claude pos1 --disallowed-tools ExitPlanMode AskUserQuestion");
    let positionals = cmd.positional_args(&[], &["disallowed-tools"]);
    assert_eq!(positionals.len(), 1);
}
