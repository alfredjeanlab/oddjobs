// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Basic lexer tests: words, operators, whitespace, newlines, edge cases.

use crate::lexer::Lexer;
use crate::token::TokenKind;

lex_tests! {
    empty_input: "" => [],
    whitespace_only: "   \t  " => [],
}

lex_tests! {
    single_word: "echo" => [TokenKind::Word("echo".into())],
    simple_words: "echo hello world" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("hello".into()),
        TokenKind::Word("world".into()),
    ],
    multiple_spaces: "ls   -la" => [
        TokenKind::Word("ls".into()),
        TokenKind::Word("-la".into()),
    ],
    tabs_and_spaces: "cmd1\t  cmd2" => [
        TokenKind::Word("cmd1".into()),
        TokenKind::Word("cmd2".into()),
    ],
    command_with_flags: "ls -la --color=auto" => [
        TokenKind::Word("ls".into()),
        TokenKind::Word("-la".into()),
        TokenKind::Word("--color=auto".into()),
    ],
    special_chars_in_words: "ls ./path/to/file.txt" => [
        TokenKind::Word("ls".into()),
        TokenKind::Word("./path/to/file.txt".into()),
    ],
}

span_tests! {
    single_word_span: "echo" => [(0, 4)],
    simple_words_span: "echo hello world" => [(0, 4), (5, 10), (11, 16)],
}

lex_tests! {
    and_operator: "cmd1 && cmd2" => [
        TokenKind::Word("cmd1".into()),
        TokenKind::And,
        TokenKind::Word("cmd2".into()),
    ],
    or_operator: "cmd1 || cmd2" => [
        TokenKind::Word("cmd1".into()),
        TokenKind::Or,
        TokenKind::Word("cmd2".into()),
    ],
    pipe_operator: "ls | grep foo" => [
        TokenKind::Word("ls".into()),
        TokenKind::Pipe,
        TokenKind::Word("grep".into()),
        TokenKind::Word("foo".into()),
    ],
    semicolon_operator: "cmd1 ; cmd2" => [
        TokenKind::Word("cmd1".into()),
        TokenKind::Semi,
        TokenKind::Word("cmd2".into()),
    ],
    background_operator: "sleep 10 &" => [
        TokenKind::Word("sleep".into()),
        TokenKind::Word("10".into()),
        TokenKind::Ampersand,
    ],
    operators_without_spaces: "cmd1&&cmd2" => [
        TokenKind::Word("cmd1".into()),
        TokenKind::And,
        TokenKind::Word("cmd2".into()),
    ],
    pipe_without_spaces: "ls|grep" => [
        TokenKind::Word("ls".into()),
        TokenKind::Pipe,
        TokenKind::Word("grep".into()),
    ],
    all_operators: "a && b || c | d ; e &" => [
        TokenKind::Word("a".into()),
        TokenKind::And,
        TokenKind::Word("b".into()),
        TokenKind::Or,
        TokenKind::Word("c".into()),
        TokenKind::Pipe,
        TokenKind::Word("d".into()),
        TokenKind::Semi,
        TokenKind::Word("e".into()),
        TokenKind::Ampersand,
    ],
    only_operators: "&&||;" => [
        TokenKind::And,
        TokenKind::Or,
        TokenKind::Semi,
    ],
}

span_tests! {
    and_operator_span: "cmd1 && cmd2" => [(0, 4), (5, 7), (8, 12)],
    or_operator_span: "cmd1 || cmd2" => [(0, 4), (5, 7), (8, 12)],
    pipe_operator_span: "ls | grep" => [(0, 2), (3, 4), (5, 9)],
    semicolon_operator_span: "cmd1 ; cmd2" => [(0, 4), (5, 6), (7, 11)],
    background_operator_span: "sleep &" => [(0, 5), (6, 7)],
}

lex_tests! {
    newline_separator: "cmd1\ncmd2" => [
        TokenKind::Word("cmd1".into()),
        TokenKind::Newline,
        TokenKind::Word("cmd2".into()),
    ],
    multiple_newlines: "cmd1\n\n\ncmd2" => [
        TokenKind::Word("cmd1".into()),
        TokenKind::Newline,
        TokenKind::Word("cmd2".into()),
    ],
    windows_newlines: "cmd1\r\ncmd2" => [
        TokenKind::Word("cmd1".into()),
        TokenKind::Newline,
        TokenKind::Word("cmd2".into()),
    ],
    trailing_newline: "cmd\n" => [
        TokenKind::Word("cmd".into()),
        TokenKind::Newline,
    ],
    carriage_return_alone: "cmd1\rcmd2" => [
        TokenKind::Word("cmd1".into()),
        TokenKind::Newline,
        TokenKind::Word("cmd2".into()),
    ],
    mixed_line_endings: "a\nb\r\nc\rd" => [
        TokenKind::Word("a".into()),
        TokenKind::Newline,
        TokenKind::Word("b".into()),
        TokenKind::Newline,
        TokenKind::Word("c".into()),
        TokenKind::Newline,
        TokenKind::Word("d".into()),
    ],
    newlines_only: "\n\n\n" => [TokenKind::Newline],
    spaces_between_newlines: "\n  \n  \n" => [TokenKind::Newline],
}

span_tests! {
    crlf_span_accuracy: "a\r\nb" => [(0, 1), (1, 3), (3, 4)],
    bare_cr_span_accuracy: "cmd1\rcmd2" => [(0, 4), (4, 5), (5, 9)],
}

lex_tests! {
    // Between tokens: backslash-newline joins lines
    line_continuation_between_words: "echo \\\nhello" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("hello".into()),
    ],
    line_continuation_with_spaces: "echo \\\n  hello" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("hello".into()),
    ],
    // Within a word: backslash-newline joins without space
    line_continuation_within_word: "hel\\\nlo" => [
        TokenKind::Word("hello".into()),
    ],
    // CRLF line continuation
    line_continuation_crlf: "echo \\\r\nhello" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("hello".into()),
    ],
    line_continuation_within_word_crlf: "hel\\\r\nlo" => [
        TokenKind::Word("hello".into()),
    ],
    // Multiple continuations
    multiple_line_continuations: "a \\\nb \\\nc" => [
        TokenKind::Word("a".into()),
        TokenKind::Word("b".into()),
        TokenKind::Word("c".into()),
    ],
    // Continuation at start of input
    line_continuation_at_start: "\\\necho" => [
        TokenKind::Word("echo".into()),
    ],
    // Backslash escapes the next character in unquoted context
    backslash_in_word: "foo\\\\bar" => [
        TokenKind::Word("foo\\bar".into()),
    ],
    // Backslash-space escapes the space (joins words)
    backslash_space: "foo\\ bar" => [
        TokenKind::Word("foo bar".into()),
    ],
    // Backslash before special characters produces literals
    backslash_open_paren: "echo \\(" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("(".into()),
    ],
    backslash_close_paren: "echo \\)" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word(")".into()),
    ],
    backslash_semicolon: "echo \\;" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word(";".into()),
    ],
    backslash_pipe: "echo \\|" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("|".into()),
    ],
    backslash_ampersand: "echo \\&" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("&".into()),
    ],
    backslash_double_backslash: "echo \\\\" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("\\".into()),
    ],
    // find command with escaped parens
    find_escaped_parens: "find . \\( -name foo \\)" => [
        TokenKind::Word("find".into()),
        TokenKind::Word(".".into()),
        TokenKind::Word("(".into()),
        TokenKind::Word("-name".into()),
        TokenKind::Word("foo".into()),
        TokenKind::Word(")".into()),
    ],
    // Backslash before regular character strips the backslash
    backslash_regular_char: "\\a" => [
        TokenKind::Word("a".into()),
    ],
    // Backslash escapes braces
    backslash_open_brace: "echo \\{" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("{".into()),
    ],
    backslash_close_brace: "echo \\}" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("}".into()),
    ],
    // Backslash escapes quotes (prevents quote processing)
    backslash_single_quote: "echo \\'" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("'".into()),
    ],
    backslash_double_quote: "echo \\\"" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("\"".into()),
    ],
    // Backslash escapes redirection chars
    backslash_gt: "echo \\>" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word(">".into()),
    ],
    backslash_lt: "echo \\<" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("<".into()),
    ],
    // Backslash escapes dollar sign (prevents variable expansion)
    backslash_dollar: "echo \\$HOME" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("$HOME".into()),
    ],
    // Backslash escapes backtick (prevents command substitution)
    backslash_backtick: "echo \\`date\\`" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("`date`".into()),
    ],
    // Trailing backslash at EOF is a literal backslash
    trailing_backslash: "echo foo\\" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("foo\\".into()),
    ],
}

lex_tests! {
    unicode_in_words: "echo 你好" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("你好".into()),
    ],
    emoji_in_word: "echo 🦦" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("🦦".into()),
    ],
    multibyte_boundary_with_operator: "你好|世界" => [
        TokenKind::Word("你好".into()),
        TokenKind::Pipe,
        TokenKind::Word("世界".into()),
    ],
}

span_tests! {
    emoji_span: "echo 🦦" => [(0, 4), (5, 9)],  // emoji is 4 bytes
    chinese_span: "你好|世界" => [(0, 6), (6, 7), (7, 13)],  // 2 chars * 3 bytes each
}

//
// The lexer does NOT strip comments. The `#` character is not a word boundary,
// so it becomes part of words. Comment handling is deferred to execution time
// (if supported). These tests document the current behavior.

lex_tests! {
    // # at start of line is part of the word
    comment_line: "# comment" => [TokenKind::Word("#".into()), TokenKind::Word("comment".into())],

    // # after command is part of the next word
    comment_after_cmd: "echo hello # comment" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("hello".into()),
        TokenKind::Word("#".into()),
        TokenKind::Word("comment".into()),
    ],

    // # inside single quotes is literal
    comment_in_single_quotes: "echo '#'" => [
        TokenKind::Word("echo".into()),
        TokenKind::SingleQuoted("#".into()),
    ],

    // # inside double quotes is literal
    comment_in_double_quotes: "echo \"#\"" => [
        TokenKind::Word("echo".into()),
        TokenKind::DoubleQuoted(vec![crate::ast::WordPart::double_quoted("#")]),
    ],

    // # adjacent to word
    comment_adjacent: "foo#bar" => [TokenKind::Word("foo#bar".into())],

    // Word starting with #
    hash_word: "#tag" => [TokenKind::Word("#tag".into())],
}

//
// The lexer does NOT handle tilde expansion specially. The `~` character is
// treated as part of words. Tilde expansion is handled at execution time.

lex_tests! {
    // ~ at start is part of word
    tilde_home: "~/path" => [TokenKind::Word("~/path".into())],

    // ~user form is part of word
    tilde_user: "~user/path" => [TokenKind::Word("~user/path".into())],

    // ~ in middle of word
    tilde_in_word: "foo~bar" => [TokenKind::Word("foo~bar".into())],

    // ~ standalone
    tilde_standalone: "~" => [TokenKind::Word("~".into())],

    // ~ with variable after (common pattern: ~$USER)
    tilde_with_variable: "~$USER" => [
        TokenKind::Word("~".into()),
        TokenKind::Variable { name: "USER".into(), modifier: None },
    ],

    // ~ in command
    tilde_in_command: "cd ~" => [
        TokenKind::Word("cd".into()),
        TokenKind::Word("~".into()),
    ],

    // Home directory expansion pattern
    tilde_home_subdir: "ls ~/Documents" => [
        TokenKind::Word("ls".into()),
        TokenKind::Word("~/Documents".into()),
    ],
}

//
// Glob metacharacters (*, ?, []) are NOT expanded by the lexer. They are part
// of words. Glob expansion happens at execution time by the shell.

lex_tests! {
    // * wildcard
    glob_star: "ls *.txt" => [
        TokenKind::Word("ls".into()),
        TokenKind::Word("*.txt".into()),
    ],

    // ? wildcard
    glob_question: "ls file?.txt" => [
        TokenKind::Word("ls".into()),
        TokenKind::Word("file?.txt".into()),
    ],

    // Character class
    glob_bracket: "ls [abc].txt" => [
        TokenKind::Word("ls".into()),
        TokenKind::Word("[abc].txt".into()),
    ],

    // Complex glob pattern
    glob_complex: "ls **/*.rs" => [
        TokenKind::Word("ls".into()),
        TokenKind::Word("**/*.rs".into()),
    ],

    // Glob in quotes (literal, not expanded)
    glob_in_single_quotes: "echo '*.txt'" => [
        TokenKind::Word("echo".into()),
        TokenKind::SingleQuoted("*.txt".into()),
    ],

    // Brace expansion pattern (not expanded by lexer)
    brace_expansion: "echo {a,b,c}" => [
        TokenKind::Word("echo".into()),
        TokenKind::LBrace,
        TokenKind::Word("a,b,c".into()),
        TokenKind::RBrace,
    ],

    // Numeric range (not expanded by lexer)
    numeric_range: "echo {1..10}" => [
        TokenKind::Word("echo".into()),
        TokenKind::LBrace,
        TokenKind::Word("1..10".into()),
        TokenKind::RBrace,
    ],

    // Escaped glob metacharacters preserve backslash for expansion phase
    backslash_asterisk: "\\*" => [TokenKind::Word("\\*".into())],
    backslash_question: "\\?" => [TokenKind::Word("\\?".into())],
    backslash_bracket: "\\[" => [TokenKind::Word("\\[".into())],
    // Double backslash before asterisk: first backslash escapes second
    double_backslash_asterisk: "\\\\*" => [TokenKind::Word("\\*".into())],
}

lex_tests! {
    triple_ampersand: "&&&" => [TokenKind::And, TokenKind::Ampersand],
    quadruple_pipe: "||||" => [TokenKind::Or, TokenKind::Or],
    double_semicolon: ";;" => [TokenKind::Semi, TokenKind::Semi],
}

span_tests! {
    triple_ampersand_span: "&&&" => [(0, 2), (2, 3)],
    quadruple_pipe_span: "||||" => [(0, 2), (2, 4)],
}

lex_tests! {
    mixed_input: "echo hello && ls | grep foo" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("hello".into()),
        TokenKind::And,
        TokenKind::Word("ls".into()),
        TokenKind::Pipe,
        TokenKind::Word("grep".into()),
        TokenKind::Word("foo".into()),
    ],
}

span_tests! {
    mixed_input_span: "echo hello && ls | grep foo" => [
        (0, 4), (5, 10), (11, 13), (14, 16), (17, 18), (19, 23), (24, 27)
    ],
}

lex_tests! {
    lparen: "(" => [TokenKind::LParen],
    rparen: ")" => [TokenKind::RParen],
    lbrace: "{" => [TokenKind::LBrace],
    rbrace: "}" => [TokenKind::RBrace],
    subshell_tokens: "( cmd )" => [
        TokenKind::LParen,
        TokenKind::Word("cmd".into()),
        TokenKind::RParen,
    ],
    brace_group_tokens: "{ cmd; }" => [
        TokenKind::LBrace,
        TokenKind::Word("cmd".into()),
        TokenKind::Semi,
        TokenKind::RBrace,
    ],
    parens_and_word: "(cmd)" => [
        TokenKind::LParen,
        TokenKind::Word("cmd".into()),
        TokenKind::RParen,
    ],
    braces_no_space: "{cmd}" => [
        TokenKind::LBrace,
        TokenKind::Word("cmd".into()),
        TokenKind::RBrace,
    ],
}

span_tests! {
    lparen_span: "(" => [(0, 1)],
    rparen_span: ")" => [(0, 1)],
    subshell_span: "( cmd )" => [(0, 1), (2, 5), (6, 7)],
}

#[test]
fn test_single_char_tokens() {
    for (input, expected) in [
        ("|", TokenKind::Pipe),
        ("&", TokenKind::Ampersand),
        (";", TokenKind::Semi),
        ("(", TokenKind::LParen),
        (")", TokenKind::RParen),
        ("{", TokenKind::LBrace),
        ("}", TokenKind::RBrace),
    ] {
        let tokens = Lexer::tokenize(input).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, expected);
    }
}
