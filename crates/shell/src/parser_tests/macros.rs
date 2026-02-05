// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Test macros for shell parser tests.
//!
//! These macros reduce boilerplate in parser tests by providing
//! declarative test generation, matching the pattern in lexer_tests/macros.rs.

/// Generate parse success tests that verify command count.
///
/// # Usage
///
/// ```ignore
/// parse_tests! {
///     name: "input" => commands: N,
/// }
/// ```
///
/// # Example
///
/// ```ignore
/// parse_tests! {
///     single_command: "echo" => commands: 1,
///     two_commands: "a; b" => commands: 2,
///     empty_input: "" => commands: 0,
/// }
/// ```
macro_rules! parse_tests {
    ($($name:ident: $input:expr => commands: $count:expr),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let result = Parser::parse($input)
                    .expect(concat!("failed to parse: ", $input));
                assert_eq!(
                    result.commands.len(), $count,
                    "input: {:?}, expected {} commands, got {}",
                    $input, $count, result.commands.len()
                );
            }
        )*
    };
}

/// Generate parse error tests.
///
/// # Usage
///
/// ```ignore
/// parse_error_tests! {
///     name: "input" => ErrorVariant { .. },
/// }
/// ```
///
/// # Example
///
/// ```ignore
/// parse_error_tests! {
///     pipe_at_start: "| cmd" => ParseError::UnexpectedToken { .. },
///     and_at_end: "cmd &&" => ParseError::UnexpectedEof { .. },
/// }
/// ```
macro_rules! parse_error_tests {
    ($($name:ident: $input:expr => $error:pat),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let result = Parser::parse($input);
                assert!(
                    matches!(result, Err($error)),
                    "expected error {:?} for input {:?}, got {:?}",
                    stringify!($error), $input, result
                );
            }
        )*
    };
}

/// Generate simple command tests (single command, verifies name and arg count).
///
/// # Usage
///
/// ```ignore
/// simple_cmd_tests! {
///     name: "input" => (cmd_name, arg_count),
/// }
/// ```
///
/// # Example
///
/// ```ignore
/// simple_cmd_tests! {
///     echo_no_args: "echo" => ("echo", 0),
///     echo_one_arg: "echo hello" => ("echo", 1),
///     ls_two_args: "ls -la /tmp" => ("ls", 2),
/// }
/// ```
macro_rules! simple_cmd_tests {
    ($($name:ident: $input:expr => ($cmd:expr, $args:expr)),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let result = Parser::parse($input)
                    .expect(concat!("failed to parse: ", $input));
                assert_eq!(result.commands.len(), 1);
                let cmd = super::helpers::get_simple_command(&result.commands[0]);
                super::helpers::assert_literal(&cmd.name, $cmd);
                assert_eq!(
                    cmd.args.len(), $args,
                    "input: {:?}, expected {} args",
                    $input, $args
                );
            }
        )*
    };
}

/// Generate job tests (verifies job command count).
///
/// # Usage
///
/// ```ignore
/// job_tests! {
///     name: "input" => job_cmds: N,
/// }
/// ```
///
/// # Example
///
/// ```ignore
/// job_tests! {
///     two_pipe: "a | b" => job_cmds: 2,
///     three_pipe: "a | b | c" => job_cmds: 3,
/// }
/// ```
macro_rules! job_tests {
    ($($name:ident: $input:expr => job_cmds: $count:expr),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let result = Parser::parse($input)
                    .expect(concat!("failed to parse: ", $input));
                assert_eq!(result.commands.len(), 1);
                let job = super::helpers::get_job(&result.commands[0]);
                assert_eq!(
                    job.commands.len(), $count,
                    "input: {:?}, expected {} job commands",
                    $input, $count
                );
            }
        )*
    };
}

/// Generate span verification tests.
///
/// # Usage
///
/// ```ignore
/// parse_span_tests! {
///     name: "input" => (start, end),
/// }
/// ```
///
/// # Example
///
/// ```ignore
/// parse_span_tests! {
///     single_word: "echo" => (0, 4),
///     two_commands: "a; b" => (0, 4),
/// }
/// ```
macro_rules! parse_span_tests {
    ($($name:ident: $input:expr => ($start:expr, $end:expr)),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let result = Parser::parse($input)
                    .expect(concat!("failed to parse: ", $input));
                assert_eq!(result.span.start, $start, "input: {:?}", $input);
                assert_eq!(result.span.end, $end, "input: {:?}", $input);
            }
        )*
    };
}

pub(crate) use {
    parse_error_tests, parse_span_tests, parse_tests, job_tests, simple_cmd_tests,
};
