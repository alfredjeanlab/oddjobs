// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use crate::Parser;

#[test]
fn count_simple_commands_single() {
    let result = Parser::parse("echo hello").unwrap();
    assert_eq!(result.count_simple_commands(), 1);
}

#[test]
fn count_simple_commands_multiple() {
    let result = Parser::parse("echo a; ls -la; pwd").unwrap();
    assert_eq!(result.count_simple_commands(), 3);
}

#[test]
fn count_simple_commands_job() {
    let result = Parser::parse("a | b | c").unwrap();
    assert_eq!(result.count_simple_commands(), 3);
}

#[test]
fn count_simple_commands_logical() {
    let result = Parser::parse("a && b || c").unwrap();
    assert_eq!(result.count_simple_commands(), 3);
}

#[test]
fn count_simple_commands_mixed() {
    let result = Parser::parse("a | b; c && d").unwrap();
    assert_eq!(result.count_simple_commands(), 4);
}

#[test]
fn count_simple_commands_subshell() {
    let result = Parser::parse("(a; b)").unwrap();
    assert_eq!(result.count_simple_commands(), 2);
}

#[test]
fn collect_variables_single() {
    let result = Parser::parse("echo $HOME").unwrap();
    let vars = result.collect_variables();
    assert_eq!(vars, vec!["HOME"]);
}

#[test]
fn collect_variables_multiple() {
    let result = Parser::parse("echo $HOME $PATH $USER").unwrap();
    let vars = result.collect_variables();
    assert_eq!(vars, vec!["HOME", "PATH", "USER"]);
}

#[test]
fn collect_variables_dedupe() {
    let result = Parser::parse("echo $HOME $PATH $HOME").unwrap();
    let vars = result.collect_variables();
    assert_eq!(vars.len(), 2);
    assert!(vars.contains(&"HOME".to_string()));
    assert!(vars.contains(&"PATH".to_string()));
}

#[test]
fn collect_variables_nested() {
    let result = Parser::parse("echo $(echo $INNER)").unwrap();
    let vars = result.collect_variables();
    assert_eq!(vars, vec!["INNER"]);
}

#[test]
fn collect_variables_in_double_quotes() {
    let result = Parser::parse(r#"echo "$HOME/bin""#).unwrap();
    let vars = result.collect_variables();
    assert_eq!(vars, vec!["HOME"]);
}

#[test]
fn collect_variables_multiple_in_double_quotes() {
    let result = Parser::parse(r#"echo "hello $USER, your home is $HOME""#).unwrap();
    let vars = result.collect_variables();
    assert_eq!(vars, vec!["USER", "HOME"]);
}

#[test]
fn collect_variables_braced_in_double_quotes() {
    let result = Parser::parse(r#"echo "${HOME:-/tmp}/config""#).unwrap();
    let vars = result.collect_variables();
    assert_eq!(vars, vec!["HOME"]);
}

#[test]
fn has_command_substitutions_false() {
    let result = Parser::parse("echo hello $HOME").unwrap();
    assert!(!result.has_command_substitutions());
}

#[test]
fn has_command_substitutions_dollar() {
    let result = Parser::parse("echo $(date)").unwrap();
    assert!(result.has_command_substitutions());
}

#[test]
fn has_command_substitutions_backtick() {
    let result = Parser::parse("echo `date`").unwrap();
    assert!(result.has_command_substitutions());
}

#[test]
fn max_nesting_depth_flat() {
    let result = Parser::parse("echo hello").unwrap();
    assert_eq!(result.max_nesting_depth(), 0);
}

#[test]
fn max_nesting_depth_one() {
    let result = Parser::parse("(echo hello)").unwrap();
    assert_eq!(result.max_nesting_depth(), 1);
}

#[test]
fn max_nesting_depth_brace() {
    let result = Parser::parse("{ echo hello; }").unwrap();
    assert_eq!(result.max_nesting_depth(), 1);
}

#[test]
fn max_nesting_depth_two() {
    let result = Parser::parse("((echo hello))").unwrap();
    assert_eq!(result.max_nesting_depth(), 2);
}

#[test]
fn max_nesting_depth_three() {
    let result = Parser::parse("(((cmd)))").unwrap();
    assert_eq!(result.max_nesting_depth(), 3);
}

#[test]
fn max_nesting_depth_mixed() {
    let result = Parser::parse("{ (cmd); }").unwrap();
    assert_eq!(result.max_nesting_depth(), 2);
}
