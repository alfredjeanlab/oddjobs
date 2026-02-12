// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Redirection lexer tests: output, input, here-docs, file descriptor duplication.

use crate::lexer::{Lexer, LexerError};
use crate::token::{DupTarget, TokenKind};

lex_tests! {
    redirect_out: "echo hello > file.txt" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("hello".into()),
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("file.txt".into()),
    ],
    redirect_append: "echo hello >> file.txt" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("hello".into()),
        TokenKind::RedirectAppend { fd: None },
        TokenKind::Word("file.txt".into()),
    ],
    redirect_stderr: "cmd 2>error.log" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectOut { fd: Some(2) },
        TokenKind::Word("error.log".into()),
    ],
    redirect_stderr_append: "cmd 2>>error.log" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectAppend { fd: Some(2) },
        TokenKind::Word("error.log".into()),
    ],
    redirect_no_space: "echo>file" => [
        TokenKind::Word("echo".into()),
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("file".into()),
    ],
    redirect_fd9: "cmd 9> file" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectOut { fd: Some(9) },
        TokenKind::Word("file".into()),
    ],
    redirect_multidigit_fd: "10>file" => [
        TokenKind::RedirectOut { fd: Some(10) },
        TokenKind::Word("file".into()),
    ],
    redirect_adjacent_at_start: ">file" => [
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("file".into()),
    ],
    redirect_adjacent_multiple: ">>file<input" => [
        TokenKind::RedirectAppend { fd: None },
        TokenKind::Word("file".into()),
        TokenKind::RedirectIn { fd: None },
        TokenKind::Word("input".into()),
    ],
    redirect_triple_gt: ">>>" => [
        TokenKind::RedirectAppend { fd: None },
        TokenKind::RedirectOut { fd: None },
    ],
}

span_tests! {
    redirect_out_span: "echo hello > file.txt" => [(0, 4), (5, 10), (11, 12), (13, 21)],
    redirect_append_span: "echo hello >> file.txt" => [(0, 4), (5, 10), (11, 13), (14, 22)],
    redirect_stderr_span: "a 2>>b" => [(0, 1), (2, 5), (5, 6)],
}

lex_tests! {
    redirect_in: "cat < input.txt" => [
        TokenKind::Word("cat".into()),
        TokenKind::RedirectIn { fd: None },
        TokenKind::Word("input.txt".into()),
    ],
    redirect_in_fd: "cmd 0<input.txt" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectIn { fd: Some(0) },
        TokenKind::Word("input.txt".into()),
    ],
}

// Here-documents with body capture are tested in heredoc.rs.
// Here-strings (<<<) don't have multi-line bodies and are tested here.

lex_tests! {
    herestring: "cat <<<hello" => [
        TokenKind::Word("cat".into()),
        TokenKind::HereString { fd: None },
        TokenKind::Word("hello".into()),
    ],
    herestring_with_fd: "cmd 0<<<text" => [
        TokenKind::Word("cmd".into()),
        TokenKind::HereString { fd: Some(0) },
        TokenKind::Word("text".into()),
    ],
    herestring_standalone: "<<<word" => [
        TokenKind::HereString { fd: None },
        TokenKind::Word("word".into()),
    ],
}

lex_tests! {
    redirect_both: "cmd &>output.txt" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectBoth { append: false },
        TokenKind::Word("output.txt".into()),
    ],
    redirect_both_append: "cmd &>>output.txt" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectBoth { append: true },
        TokenKind::Word("output.txt".into()),
    ],
    redirect_to_dev_null: "cmd &>/dev/null" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectBoth { append: false },
        TokenKind::Word("/dev/null".into()),
    ],
}

lex_tests! {
    dup_stderr_to_stdout: "cmd 2>&1" => [
        TokenKind::Word("cmd".into()),
        TokenKind::DuplicateFd {
            source: 2,
            target: DupTarget::Fd(1),
            output: true,
        },
    ],
    dup_stdout_to_stderr: "cmd 1>&2" => [
        TokenKind::Word("cmd".into()),
        TokenKind::DuplicateFd {
            source: 1,
            target: DupTarget::Fd(2),
            output: true,
        },
    ],
    dup_close: "cmd 2>&-" => [
        TokenKind::Word("cmd".into()),
        TokenKind::DuplicateFd {
            source: 2,
            target: DupTarget::Close,
            output: true,
        },
    ],
    dup_input: "cmd 0<&3" => [
        TokenKind::Word("cmd".into()),
        TokenKind::DuplicateFd {
            source: 0,
            target: DupTarget::Fd(3),
            output: false,
        },
    ],
    dup_input_close: "cmd <&-" => [
        TokenKind::Word("cmd".into()),
        TokenKind::DuplicateFd {
            source: 0,
            target: DupTarget::Close,
            output: false,
        },
    ],
    dup_output_no_fd: "cmd >&2" => [
        TokenKind::Word("cmd".into()),
        TokenKind::DuplicateFd {
            source: 1,
            target: DupTarget::Fd(2),
            output: true,
        },
    ],
    dup_input_no_fd: "cmd <&3" => [
        TokenKind::Word("cmd".into()),
        TokenKind::DuplicateFd {
            source: 0,
            target: DupTarget::Fd(3),
            output: false,
        },
    ],
}

span_tests! {
    dup_span_accuracy: "a 2>&1" => [(0, 1), (2, 6)],
    dup_span_standalone: "2>&1" => [(0, 4)],
    herestring_span: "<<<word" => [(0, 3), (3, 7)],
}

lex_tests! {
    number_alone_is_word: "echo 2" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("2".into()),
    ],
    number_with_space_then_redirect: "echo 2 > file" => [
        TokenKind::Word("echo".into()),
        TokenKind::Word("2".into()),
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("file".into()),
    ],
    number_no_space_is_fd: "cmd 2>file" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectOut { fd: Some(2) },
        TokenKind::Word("file".into()),
    ],
}

#[test]
fn redirect_with_variable() {
    let tokens = Lexer::tokenize("echo > $FILE").unwrap();
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[1].kind, TokenKind::RedirectOut { fd: None });
    assert!(matches!(tokens[2].kind, TokenKind::Variable { .. }));
}

lex_tests! {
    multiple_redirects: "cmd <in >out 2>&1" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectIn { fd: None },
        TokenKind::Word("in".into()),
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("out".into()),
        TokenKind::DuplicateFd {
            source: 2,
            target: DupTarget::Fd(1),
            output: true,
        },
    ],
    pipe_with_redirect: "cmd1 | cmd2 > out" => [
        TokenKind::Word("cmd1".into()),
        TokenKind::Pipe,
        TokenKind::Word("cmd2".into()),
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("out".into()),
    ],
    redirect_then_pipe: "cmd > file | cat" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("file".into()),
        TokenKind::Pipe,
        TokenKind::Word("cat".into()),
    ],
    pipe_then_redirect: "cmd | cat > output" => [
        TokenKind::Word("cmd".into()),
        TokenKind::Pipe,
        TokenKind::Word("cat".into()),
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("output".into()),
    ],
    redirect_in_complex_command: "command 2>&1 | tee output.log" => [
        TokenKind::Word("command".into()),
        TokenKind::DuplicateFd {
            source: 2,
            target: DupTarget::Fd(1),
            output: true,
        },
        TokenKind::Pipe,
        TokenKind::Word("tee".into()),
        TokenKind::Word("output.log".into()),
    ],
    // heredoc_in_job moved to heredoc.rs (requires newline and body)
    fd_swap: "exec 3>&1 1>&2 2>&3" => [
        TokenKind::Word("exec".into()),
        TokenKind::DuplicateFd {
            source: 3,
            target: DupTarget::Fd(1),
            output: true,
        },
        TokenKind::DuplicateFd {
            source: 1,
            target: DupTarget::Fd(2),
            output: true,
        },
        TokenKind::DuplicateFd {
            source: 2,
            target: DupTarget::Fd(3),
            output: true,
        },
    ],
    complex_job: "cat < in | sort | uniq > out 2>&1" => [
        TokenKind::Word("cat".into()),
        TokenKind::RedirectIn { fd: None },
        TokenKind::Word("in".into()),
        TokenKind::Pipe,
        TokenKind::Word("sort".into()),
        TokenKind::Pipe,
        TokenKind::Word("uniq".into()),
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("out".into()),
        TokenKind::DuplicateFd {
            source: 2,
            target: DupTarget::Fd(1),
            output: true,
        },
    ],
    redirect_both_streams: "cmd > out 2>&1" => [
        TokenKind::Word("cmd".into()),
        TokenKind::RedirectOut { fd: None },
        TokenKind::Word("out".into()),
        TokenKind::DuplicateFd {
            source: 2,
            target: DupTarget::Fd(1),
            output: true,
        },
    ],
}

lex_error_tests! {
    error_dup_missing_target: "cmd >&" => LexerError::InvalidRedirection { .. },
    error_dup_missing_target_input: "cmd <&" => LexerError::InvalidRedirection { .. },
    error_dup_invalid_target: "cmd >&x" => LexerError::InvalidRedirection { .. },
}
