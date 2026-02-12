// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::token::Span;
use crate::Parser;

// ── AST type tests ───────────────────────────────────────────────────────

#[test]
fn word_literal() {
    let word = Word { parts: vec![WordPart::literal("echo")], span: Span::new(0, 4) };
    assert_eq!(word.parts.len(), 1);
    assert!(matches!(&word.parts[0], WordPart::Literal { value, .. } if value == "echo"));
}

#[test]
fn word_variable() {
    let word = Word {
        parts: vec![WordPart::Variable { name: "HOME".into(), modifier: None }],
        span: Span::new(0, 5),
    };
    assert_eq!(word.parts.len(), 1);
    assert!(matches!(
        &word.parts[0],
        WordPart::Variable { name, modifier: None } if name == "HOME"
    ));
}

#[test]
fn word_variable_with_modifier() {
    let word = Word {
        parts: vec![WordPart::Variable { name: "PATH".into(), modifier: Some(":-/bin".into()) }],
        span: Span::new(0, 14),
    };
    assert!(matches!(
        &word.parts[0],
        WordPart::Variable { name, modifier: Some(m) } if name == "PATH" && m == ":-/bin"
    ));
}

#[test]
fn word_command_substitution() {
    let date_cmd = SimpleCommand {
        env: vec![],
        name: Word { parts: vec![WordPart::literal("date")], span: Span::new(0, 4) },
        args: vec![],
        redirections: vec![],
        span: Span::new(0, 4),
    };
    let body = CommandList {
        commands: vec![AndOrList {
            first: CommandItem {
                command: Command::Simple(date_cmd),
                background: false,
                span: Span::new(0, 4),
            },
            rest: vec![],
            span: Span::new(0, 4),
        }],
        span: Span::new(0, 4),
    };

    let word = Word {
        parts: vec![WordPart::CommandSubstitution {
            body: SubstitutionBody::Parsed(Box::new(body)),
            backtick: false,
        }],
        span: Span::new(0, 7),
    };
    assert!(matches!(
        &word.parts[0],
        WordPart::CommandSubstitution { body: SubstitutionBody::Parsed(_), backtick: false }
    ));
}

#[test]
fn simple_command() {
    let cmd = SimpleCommand {
        env: vec![],
        name: Word { parts: vec![WordPart::literal("ls")], span: Span::new(0, 2) },
        args: vec![
            Word { parts: vec![WordPart::literal("-la")], span: Span::new(3, 6) },
            Word { parts: vec![WordPart::literal("/tmp")], span: Span::new(7, 11) },
        ],
        redirections: vec![],
        span: Span::new(0, 11),
    };

    assert_eq!(cmd.args.len(), 2);
    assert!(matches!(
        &cmd.name.parts[0],
        WordPart::Literal { value, .. } if value == "ls"
    ));
}

#[test]
fn command_list() {
    let cmd1 = SimpleCommand {
        env: vec![],
        name: Word { parts: vec![WordPart::literal("echo")], span: Span::new(0, 4) },
        args: vec![Word { parts: vec![WordPart::literal("a")], span: Span::new(5, 6) }],
        redirections: vec![],
        span: Span::new(0, 6),
    };

    let cmd2 = SimpleCommand {
        env: vec![],
        name: Word { parts: vec![WordPart::literal("echo")], span: Span::new(9, 13) },
        args: vec![Word { parts: vec![WordPart::literal("b")], span: Span::new(14, 15) }],
        redirections: vec![],
        span: Span::new(9, 15),
    };

    let list = CommandList {
        commands: vec![
            AndOrList {
                first: CommandItem {
                    command: Command::Simple(cmd1),
                    background: false,
                    span: Span::new(0, 6),
                },
                rest: vec![],
                span: Span::new(0, 6),
            },
            AndOrList {
                first: CommandItem {
                    command: Command::Simple(cmd2),
                    background: false,
                    span: Span::new(9, 15),
                },
                rest: vec![],
                span: Span::new(9, 15),
            },
        ],
        span: Span::new(0, 15),
    };

    assert_eq!(list.commands.len(), 2);
}

#[test]
fn span_merge_for_command() {
    let start_span = Span::new(0, 4);
    let end_span = Span::new(10, 15);
    let merged = start_span.merge(end_span);
    assert_eq!(merged.start, 0);
    assert_eq!(merged.end, 15);
}

#[test]
fn command_equality() {
    let cmd1 = Command::Simple(SimpleCommand {
        env: vec![],
        name: Word { parts: vec![WordPart::literal("echo")], span: Span::new(0, 4) },
        args: vec![],
        redirections: vec![],
        span: Span::new(0, 4),
    });

    let cmd2 = Command::Simple(SimpleCommand {
        env: vec![],
        name: Word { parts: vec![WordPart::literal("echo")], span: Span::new(0, 4) },
        args: vec![],
        redirections: vec![],
        span: Span::new(0, 4),
    });

    assert_eq!(cmd1, cmd2);
}

#[test]
fn word_part_equality() {
    let part1 = WordPart::literal("hello");
    let part2 = WordPart::literal("hello");
    let part3 = WordPart::literal("world");

    assert_eq!(part1, part2);
    assert_ne!(part1, part3);
}

#[test]
fn job() {
    let job = Job {
        commands: vec![
            SimpleCommand {
                env: vec![],
                name: Word { parts: vec![WordPart::literal("cat")], span: Span::new(0, 3) },
                args: vec![Word { parts: vec![WordPart::literal("file")], span: Span::new(4, 8) }],
                redirections: vec![],
                span: Span::new(0, 8),
            },
            SimpleCommand {
                env: vec![],
                name: Word { parts: vec![WordPart::literal("grep")], span: Span::new(11, 15) },
                args: vec![Word {
                    parts: vec![WordPart::literal("pattern")],
                    span: Span::new(16, 23),
                }],
                redirections: vec![],
                span: Span::new(11, 23),
            },
        ],
        span: Span::new(0, 23),
    };

    assert_eq!(job.commands.len(), 2);
}

#[test]
fn and_or_list() {
    let first = CommandItem {
        command: Command::Simple(SimpleCommand {
            env: vec![],
            name: Word { parts: vec![WordPart::literal("cmd1")], span: Span::new(0, 4) },
            args: vec![],
            redirections: vec![],
            span: Span::new(0, 4),
        }),
        background: false,
        span: Span::new(0, 4),
    };

    let second = CommandItem {
        command: Command::Simple(SimpleCommand {
            env: vec![],
            name: Word { parts: vec![WordPart::literal("cmd2")], span: Span::new(8, 12) },
            args: vec![],
            redirections: vec![],
            span: Span::new(8, 12),
        }),
        background: false,
        span: Span::new(8, 12),
    };

    let and_or = AndOrList { first, rest: vec![(LogicalOp::And, second)], span: Span::new(0, 12) };

    assert_eq!(and_or.rest.len(), 1);
    assert_eq!(and_or.rest[0].0, LogicalOp::And);
}

#[test]
fn command_item_background() {
    let item = CommandItem {
        command: Command::Simple(SimpleCommand {
            env: vec![],
            name: Word { parts: vec![WordPart::literal("sleep")], span: Span::new(0, 5) },
            args: vec![Word { parts: vec![WordPart::literal("10")], span: Span::new(6, 8) }],
            redirections: vec![],
            span: Span::new(0, 8),
        }),
        background: true,
        span: Span::new(0, 10),
    };

    assert!(item.background);
}

#[test]
fn logical_op() {
    assert_eq!(LogicalOp::And, LogicalOp::And);
    assert_eq!(LogicalOp::Or, LogicalOp::Or);
    assert_ne!(LogicalOp::And, LogicalOp::Or);
}

// ── CLI argument parsing tests ───────────────────────────────────────────

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

// ── Utility method tests ─────────────────────────────────────────────────

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
