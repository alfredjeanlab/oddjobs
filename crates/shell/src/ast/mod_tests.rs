// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use crate::token::Span;

#[test]
fn word_literal() {
    let word = Word {
        parts: vec![WordPart::literal("echo")],
        span: Span::new(0, 4),
    };
    assert_eq!(word.parts.len(), 1);
    assert!(matches!(&word.parts[0], WordPart::Literal { value, .. } if value == "echo"));
}

#[test]
fn word_variable() {
    let word = Word {
        parts: vec![WordPart::Variable {
            name: "HOME".into(),
            modifier: None,
        }],
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
        parts: vec![WordPart::Variable {
            name: "PATH".into(),
            modifier: Some(":-/bin".into()),
        }],
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
        name: Word {
            parts: vec![WordPart::literal("date")],
            span: Span::new(0, 4),
        },
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
        WordPart::CommandSubstitution {
            body: SubstitutionBody::Parsed(_),
            backtick: false
        }
    ));
}

#[test]
fn simple_command() {
    let cmd = SimpleCommand {
        env: vec![],
        name: Word {
            parts: vec![WordPart::literal("ls")],
            span: Span::new(0, 2),
        },
        args: vec![
            Word {
                parts: vec![WordPart::literal("-la")],
                span: Span::new(3, 6),
            },
            Word {
                parts: vec![WordPart::literal("/tmp")],
                span: Span::new(7, 11),
            },
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
        name: Word {
            parts: vec![WordPart::literal("echo")],
            span: Span::new(0, 4),
        },
        args: vec![Word {
            parts: vec![WordPart::literal("a")],
            span: Span::new(5, 6),
        }],
        redirections: vec![],
        span: Span::new(0, 6),
    };

    let cmd2 = SimpleCommand {
        env: vec![],
        name: Word {
            parts: vec![WordPart::literal("echo")],
            span: Span::new(9, 13),
        },
        args: vec![Word {
            parts: vec![WordPart::literal("b")],
            span: Span::new(14, 15),
        }],
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
        name: Word {
            parts: vec![WordPart::literal("echo")],
            span: Span::new(0, 4),
        },
        args: vec![],
        redirections: vec![],
        span: Span::new(0, 4),
    });

    let cmd2 = Command::Simple(SimpleCommand {
        env: vec![],
        name: Word {
            parts: vec![WordPart::literal("echo")],
            span: Span::new(0, 4),
        },
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
fn pipeline() {
    let pipeline = Pipeline {
        commands: vec![
            SimpleCommand {
                env: vec![],
                name: Word {
                    parts: vec![WordPart::literal("cat")],
                    span: Span::new(0, 3),
                },
                args: vec![Word {
                    parts: vec![WordPart::literal("file")],
                    span: Span::new(4, 8),
                }],
                redirections: vec![],
                span: Span::new(0, 8),
            },
            SimpleCommand {
                env: vec![],
                name: Word {
                    parts: vec![WordPart::literal("grep")],
                    span: Span::new(11, 15),
                },
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

    assert_eq!(pipeline.commands.len(), 2);
}

#[test]
fn and_or_list() {
    let first = CommandItem {
        command: Command::Simple(SimpleCommand {
            env: vec![],
            name: Word {
                parts: vec![WordPart::literal("cmd1")],
                span: Span::new(0, 4),
            },
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
            name: Word {
                parts: vec![WordPart::literal("cmd2")],
                span: Span::new(8, 12),
            },
            args: vec![],
            redirections: vec![],
            span: Span::new(8, 12),
        }),
        background: false,
        span: Span::new(8, 12),
    };

    let and_or = AndOrList {
        first,
        rest: vec![(LogicalOp::And, second)],
        span: Span::new(0, 12),
    };

    assert_eq!(and_or.rest.len(), 1);
    assert_eq!(and_or.rest[0].0, LogicalOp::And);
}

#[test]
fn command_item_background() {
    let item = CommandItem {
        command: Command::Simple(SimpleCommand {
            env: vec![],
            name: Word {
                parts: vec![WordPart::literal("sleep")],
                span: Span::new(0, 5),
            },
            args: vec![Word {
                parts: vec![WordPart::literal("10")],
                span: Span::new(6, 8),
            }],
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
