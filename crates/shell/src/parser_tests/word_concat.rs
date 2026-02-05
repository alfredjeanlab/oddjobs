// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for word concatenation in the parser.
//!
//! Shell commands like `foo$VAR/bin` should produce a single Word with multiple
//! parts (`[Literal("foo"), Variable("VAR"), Literal("/bin")]`), not separate words.

use super::helpers::get_simple_command;
use crate::ast::{SubstitutionBody, WordPart};
use crate::parser::Parser;
use crate::token::Span;

// =============================================================================
// Step 2: Adjacent Token Merging Patterns
// =============================================================================

#[test]
fn literal_followed_by_variable() {
    // foo$VAR → Word([Lit("foo"), Var("VAR")])
    let result = Parser::parse("echo foo$VAR").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![
            WordPart::literal("foo"),
            WordPart::Variable {
                name: "VAR".into(),
                modifier: None
            }
        ]
    );
}

#[test]
fn variable_followed_by_literal() {
    // $VAR.txt → Word([Var("VAR"), Lit(".txt")])
    let result = Parser::parse("echo $VAR.txt").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![
            WordPart::Variable {
                name: "VAR".into(),
                modifier: None
            },
            WordPart::literal(".txt")
        ]
    );
}

#[test]
fn literal_substitution_literal() {
    // pre$(cmd)suf → Word([Lit("pre"), CmdSubst("cmd"), Lit("suf")])
    let result = Parser::parse("echo pre$(date)suf").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(cmd.args[0].parts.len(), 3);

    assert_eq!(cmd.args[0].parts[0], WordPart::literal("pre"));

    let WordPart::CommandSubstitution {
        body: SubstitutionBody::Parsed(body),
        backtick,
    } = &cmd.args[0].parts[1]
    else {
        panic!("expected command substitution");
    };
    assert!(!backtick);
    assert_eq!(body.commands.len(), 1);

    assert_eq!(cmd.args[0].parts[2], WordPart::literal("suf"));
}

#[test]
fn variable_in_middle() {
    // pre${VAR}suf → Word([Lit("pre"), Var("VAR"), Lit("suf")])
    let result = Parser::parse("echo pre${VAR}suf").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![
            WordPart::literal("pre"),
            WordPart::Variable {
                name: "VAR".into(),
                modifier: None
            },
            WordPart::literal("suf")
        ]
    );
}

#[test]
fn multiple_variables_concatenated() {
    // $A$B$C → Word([Var("A"), Var("B"), Var("C")])
    let result = Parser::parse("echo $A$B$C").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![
            WordPart::Variable {
                name: "A".into(),
                modifier: None
            },
            WordPart::Variable {
                name: "B".into(),
                modifier: None
            },
            WordPart::Variable {
                name: "C".into(),
                modifier: None
            }
        ]
    );
}

#[test]
fn path_with_variable_prefix() {
    // $HOME/bin → Word([Var("HOME"), Lit("/bin")])
    let result = Parser::parse("echo $HOME/bin").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![
            WordPart::Variable {
                name: "HOME".into(),
                modifier: None
            },
            WordPart::literal("/bin")
        ]
    );
}

#[test]
fn path_with_variable_middle() {
    // /usr/${LOCAL}/bin → Word([Lit("/usr/"), Var("LOCAL"), Lit("/bin")])
    let result = Parser::parse("echo /usr/${LOCAL}/bin").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![
            WordPart::literal("/usr/"),
            WordPart::Variable {
                name: "LOCAL".into(),
                modifier: None
            },
            WordPart::literal("/bin")
        ]
    );
}

#[test]
fn file_extension_pattern() {
    // file$NUM.log → Word([Lit("file"), Var("NUM"), Lit(".log")])
    let result = Parser::parse("echo file$NUM.log").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![
            WordPart::literal("file"),
            WordPart::Variable {
                name: "NUM".into(),
                modifier: None
            },
            WordPart::literal(".log")
        ]
    );
}

#[test]
fn braced_variable_in_word() {
    // ${NAME}_suffix → Word([Var("NAME"), Lit("_suffix")])
    let result = Parser::parse("echo ${NAME}_suffix").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![
            WordPart::Variable {
                name: "NAME".into(),
                modifier: None
            },
            WordPart::literal("_suffix")
        ]
    );
}

#[test]
fn multiple_separate_concatenated_words() {
    // Multiple arguments, each concatenated
    let result = Parser::parse("cmd $A$B foo$C").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 2);
    // First arg: $A$B
    assert_eq!(
        cmd.args[0].parts,
        vec![
            WordPart::Variable {
                name: "A".into(),
                modifier: None
            },
            WordPart::Variable {
                name: "B".into(),
                modifier: None
            }
        ]
    );
    // Second arg: foo$C
    assert_eq!(
        cmd.args[1].parts,
        vec![
            WordPart::literal("foo"),
            WordPart::Variable {
                name: "C".into(),
                modifier: None
            }
        ]
    );
}

// =============================================================================
// Step 3: Whitespace Separation
// =============================================================================

#[test]
fn whitespace_separates_words() {
    // $VAR .txt → 2 Words: Word([Var("VAR")]), Word([Lit(".txt")])
    let result = Parser::parse("echo $VAR .txt").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 2);
    assert_eq!(
        cmd.args[0].parts,
        vec![WordPart::Variable {
            name: "VAR".into(),
            modifier: None
        }]
    );
    assert_eq!(cmd.args[1].parts, vec![WordPart::literal(".txt")]);
}

#[test]
fn tab_separates_words() {
    // foo\t$VAR → 2 Words
    let result = Parser::parse("echo foo\t$VAR").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 2);
    assert_eq!(cmd.args[0].parts, vec![WordPart::literal("foo")]);
    assert_eq!(
        cmd.args[1].parts,
        vec![WordPart::Variable {
            name: "VAR".into(),
            modifier: None
        }]
    );
}

#[test]
fn multiple_spaces_separate() {
    // a    b → 2 Words (extra spaces don't matter)
    let result = Parser::parse("echo a    b").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 2);
    assert_eq!(cmd.args[0].parts, vec![WordPart::literal("a")]);
    assert_eq!(cmd.args[1].parts, vec![WordPart::literal("b")]);
}

#[test]
fn concatenation_stops_at_pipe() {
    // foo$VAR|bar → Job with 2 commands, each a single word
    let result = Parser::parse("foo$VAR|bar").unwrap();
    assert_eq!(result.commands.len(), 1);

    let cmd = &result.commands[0].first.command;
    let job = match cmd {
        crate::ast::Command::Job(p) => p,
        _ => panic!("expected job"),
    };

    assert_eq!(job.commands.len(), 2);
    assert_eq!(
        job.commands[0].name.parts,
        vec![
            WordPart::literal("foo"),
            WordPart::Variable {
                name: "VAR".into(),
                modifier: None
            }
        ]
    );
    assert_eq!(
        job.commands[1].name.parts,
        vec![WordPart::literal("bar")]
    );
}

#[test]
fn concatenation_stops_at_semicolon() {
    // foo$VAR;bar → 2 commands
    let result = Parser::parse("foo$VAR;bar").unwrap();
    assert_eq!(result.commands.len(), 2);
}

#[test]
fn concatenation_stops_at_and() {
    // foo$VAR&&bar → And chain
    let result = Parser::parse("foo$VAR&&bar").unwrap();
    assert_eq!(result.commands.len(), 1);
    assert_eq!(result.commands[0].rest.len(), 1);
}

// =============================================================================
// Step 4: Span Calculation
// =============================================================================

#[test]
fn span_covers_concatenated_word() {
    // Input: "echo foo$VAR" (12 chars: echo=0-4, space=4, foo=5-8, $VAR=8-12)
    // Expected span for "foo$VAR": Span::new(5, 12)
    let result = Parser::parse("echo foo$VAR").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args[0].span.start, 5);
    assert_eq!(cmd.args[0].span.end, 12);
}

#[test]
fn span_covers_path_pattern() {
    // Input: "echo $HOME/bin" (14 chars: echo=0-4, space=4, $HOME=5-10, /bin=10-14)
    // Expected span for "$HOME/bin": Span::new(5, 14)
    let result = Parser::parse("echo $HOME/bin").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args[0].span.start, 5);
    assert_eq!(cmd.args[0].span.end, 14);
}

#[test]
fn span_accuracy_with_braced_variable() {
    // Input: "echo ${VAR}suffix" (17 chars: echo=0-4, space=4, ${VAR}=5-11, suffix=11-17)
    // Expected span: Span::new(5, 17)
    let result = Parser::parse("echo ${VAR}suffix").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args[0].span.start, 5);
    assert_eq!(cmd.args[0].span.end, 17);
}

#[test]
fn span_covers_three_part_word() {
    // Input: "echo pre${VAR}suf" (17 chars: echo=0-4, space=4, pre=5-8, ${VAR}=8-14, suf=14-17)
    // Expected span: Span::new(5, 17)
    let result = Parser::parse("echo pre${VAR}suf").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args[0].span.start, 5);
    assert_eq!(cmd.args[0].span.end, 17);
}

#[test]
fn span_covers_entire_concatenated_command_name() {
    // "foo$VAR" as command name should span 0..7
    let result = Parser::parse("foo$VAR").unwrap();
    let cmd = get_simple_command(&result.commands[0]);
    assert_eq!(cmd.name.span, Span::new(0, 7));
}

// =============================================================================
// Step 5: Edge Cases and Integration
// =============================================================================

#[test]
fn single_literal_word() {
    // "hello" → Word([Lit("hello")])
    let result = Parser::parse("echo hello").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(cmd.args[0].parts, vec![WordPart::literal("hello")]);
}

#[test]
fn single_variable_word() {
    // "$HOME" → Word([Var("HOME")])
    let result = Parser::parse("echo $HOME").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(
        cmd.args[0].parts,
        vec![WordPart::Variable {
            name: "HOME".into(),
            modifier: None
        }]
    );
}

#[test]
fn job_with_concatenation() {
    // echo $HOME/bin | cat → Job with correct word parts
    let result = Parser::parse("echo $HOME/bin | cat").unwrap();
    let cmd = &result.commands[0].first.command;
    let job = match cmd {
        crate::ast::Command::Job(p) => p,
        _ => panic!("expected job"),
    };

    assert_eq!(job.commands.len(), 2);
    // First command has concatenated arg
    assert_eq!(
        job.commands[0].args[0].parts,
        vec![
            WordPart::Variable {
                name: "HOME".into(),
                modifier: None
            },
            WordPart::literal("/bin")
        ]
    );
}

#[test]
fn and_or_with_concatenation() {
    // test$A && echo$B → AndOrList with concatenated words
    let result = Parser::parse("test$A && echo$B").unwrap();
    assert_eq!(result.commands.len(), 1);
    assert_eq!(result.commands[0].rest.len(), 1);

    // First command name is "test$A"
    let first_and_or = crate::ast::AndOrList {
        first: result.commands[0].first.clone(),
        rest: vec![],
        span: result.commands[0].first.span,
    };
    let first_cmd = get_simple_command(&first_and_or);
    assert_eq!(
        first_cmd.name.parts,
        vec![
            WordPart::literal("test"),
            WordPart::Variable {
                name: "A".into(),
                modifier: None
            }
        ]
    );
}

#[test]
fn backtick_substitution_concatenation() {
    // `pwd`/bin → Word([CmdSubst(backtick), Lit("/bin")])
    let result = Parser::parse("echo `pwd`/bin").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(cmd.args[0].parts.len(), 2);

    let WordPart::CommandSubstitution { backtick, .. } = &cmd.args[0].parts[0] else {
        panic!("expected command substitution");
    };
    assert!(backtick);

    assert_eq!(cmd.args[0].parts[1], WordPart::literal("/bin"));
}

#[test]
fn visitor_finds_concatenated_variables() {
    let ast = Parser::parse("echo $A$B$C").unwrap();
    let vars = ast.collect_variables();
    assert_eq!(vars, vec!["A", "B", "C"]);
}

#[test]
fn collect_variables_from_concatenated_word() {
    // Ensure the AST visitor finds variables in concatenated words
    let result = Parser::parse("echo prefix$VAR1$VAR2/suffix").unwrap();
    let vars = result.collect_variables();
    assert_eq!(vars, vec!["VAR1", "VAR2"]);
}

#[test]
fn concatenation_in_job() {
    // Variables in piped commands
    let result = Parser::parse("echo $A$B | cat $C$D").unwrap();
    let vars = result.collect_variables();
    assert_eq!(vars, vec!["A", "B", "C", "D"]);
}

#[test]
fn has_command_substitutions_in_concatenated_word() {
    let ast = Parser::parse("echo pre$(date)suf").unwrap();
    assert!(ast.has_command_substitutions());
}

#[test]
fn command_name_with_concatenation() {
    // $HOME/bin/cmd → command name is concatenated
    let result = Parser::parse("$HOME/bin/cmd arg").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(
        cmd.name.parts,
        vec![
            WordPart::Variable {
                name: "HOME".into(),
                modifier: None
            },
            WordPart::literal("/bin/cmd")
        ]
    );
    assert_eq!(cmd.args.len(), 1);
}

// =============================================================================
// Step 6: Quote Style Preservation
// =============================================================================

#[test]
fn unquoted_literal_has_unquoted_style() {
    use crate::ast::QuoteStyle;

    let result = Parser::parse("echo hello").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    match &cmd.args[0].parts[0] {
        WordPart::Literal { value, quoted } => {
            assert_eq!(value, "hello");
            assert_eq!(*quoted, QuoteStyle::Unquoted);
        }
        _ => panic!("expected literal"),
    }
}

#[test]
fn single_quoted_literal_has_single_style() {
    use crate::ast::QuoteStyle;

    let result = Parser::parse("echo 'hello'").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    match &cmd.args[0].parts[0] {
        WordPart::Literal { value, quoted } => {
            assert_eq!(value, "hello");
            assert_eq!(*quoted, QuoteStyle::Single);
        }
        _ => panic!("expected literal"),
    }
}

#[test]
fn double_quoted_literal_has_double_style() {
    use crate::ast::QuoteStyle;

    let result = Parser::parse(r#"echo "hello""#).unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    match &cmd.args[0].parts[0] {
        WordPart::Literal { value, quoted } => {
            assert_eq!(value, "hello");
            assert_eq!(*quoted, QuoteStyle::Double);
        }
        _ => panic!("expected literal"),
    }
}

#[test]
fn single_quoted_preserves_dollar_sign() {
    use crate::ast::QuoteStyle;

    // '$VAR' should be a literal, not a variable reference
    let result = Parser::parse("echo '$VAR'").unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    match &cmd.args[0].parts[0] {
        WordPart::Literal { value, quoted } => {
            assert_eq!(value, "$VAR");
            assert_eq!(*quoted, QuoteStyle::Single);
        }
        _ => panic!("expected literal, got {:?}", cmd.args[0].parts[0]),
    }
}

#[test]
fn double_quoted_string_with_variable() {
    use crate::ast::QuoteStyle;

    // "$HOME/bin" should have Double-quoted literal parts
    // Note: boundary literals are emitted for word splitting support
    let result = Parser::parse(r#"echo "$HOME/bin""#).unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(cmd.args[0].parts.len(), 3);

    // First part is an empty boundary literal
    match &cmd.args[0].parts[0] {
        WordPart::Literal { value, quoted } => {
            assert_eq!(value, "");
            assert_eq!(*quoted, QuoteStyle::Double);
        }
        _ => panic!("expected boundary literal"),
    }

    // Second part is the variable
    assert!(matches!(
        &cmd.args[0].parts[1],
        WordPart::Variable { name, .. } if name == "HOME"
    ));

    // Third part is a double-quoted literal
    match &cmd.args[0].parts[2] {
        WordPart::Literal { value, quoted } => {
            assert_eq!(value, "/bin");
            assert_eq!(*quoted, QuoteStyle::Double);
        }
        _ => panic!("expected literal"),
    }
}

#[test]
fn mixed_quoting_styles_preserved() {
    use crate::ast::QuoteStyle;

    // hello'world'"test" → three parts with different quote styles
    let result = Parser::parse(r#"echo hello'world'"test""#).unwrap();
    let cmd = get_simple_command(&result.commands[0]);

    assert_eq!(cmd.args.len(), 1);
    assert_eq!(cmd.args[0].parts.len(), 3);

    // First: unquoted "hello"
    match &cmd.args[0].parts[0] {
        WordPart::Literal { value, quoted } => {
            assert_eq!(value, "hello");
            assert_eq!(*quoted, QuoteStyle::Unquoted);
        }
        _ => panic!("expected literal"),
    }

    // Second: single-quoted "world"
    match &cmd.args[0].parts[1] {
        WordPart::Literal { value, quoted } => {
            assert_eq!(value, "world");
            assert_eq!(*quoted, QuoteStyle::Single);
        }
        _ => panic!("expected literal"),
    }

    // Third: double-quoted "test"
    match &cmd.args[0].parts[2] {
        WordPart::Literal { value, quoted } => {
            assert_eq!(value, "test");
            assert_eq!(*quoted, QuoteStyle::Double);
        }
        _ => panic!("expected literal"),
    }
}
