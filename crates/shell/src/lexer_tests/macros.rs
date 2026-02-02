// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Test macros for shell lexer tests.
//!
//! These macros reduce boilerplate in lexer tests by providing
//! declarative test generation.

/// Generate tokenization success tests.
///
/// # Usage
///
/// ```ignore
/// lex_tests! {
///     name: "input" => [token1, token2, ...],
/// }
/// ```
///
/// # Example
///
/// ```ignore
/// lex_tests! {
///     empty_input: "" => [],
///     single_word: "echo" => [TokenKind::Word("echo".into())],
///     two_words: "echo hello" => [
///         TokenKind::Word("echo".into()),
///         TokenKind::Word("hello".into()),
///     ],
/// }
/// ```
macro_rules! lex_tests {
    ($($name:ident: $input:expr => [$($token:expr),* $(,)?]),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let tokens = Lexer::tokenize($input).expect(concat!("failed to tokenize: ", $input));
                let expected: Vec<TokenKind> = vec![$($token),*];
                let actual: Vec<_> = tokens.iter().map(|t| t.kind.clone()).collect();
                assert_eq!(actual, expected, "input: {:?}", $input);
            }
        )*
    };
}

/// Generate tokenization error tests.
///
/// # Usage
///
/// ```ignore
/// lex_error_tests! {
///     name: "input" => ErrorVariant { field: value },
/// }
/// ```
///
/// # Example
///
/// ```ignore
/// lex_error_tests! {
///     unterminated_sq: "'" => LexerError::UnterminatedSingleQuote { .. },
///     empty_var: "$" => LexerError::EmptyVariable { .. },
/// }
/// ```
macro_rules! lex_error_tests {
    ($($name:ident: $input:expr => $error:pat),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let result = Lexer::tokenize($input);
                assert!(
                    matches!(result, Err($error)),
                    "expected error {:?} for input {:?}, got {:?}",
                    stringify!($error), $input, result
                );
            }
        )*
    };
}

/// Generate span accuracy tests.
///
/// # Usage
///
/// ```ignore
/// span_tests! {
///     name: "input" => [(start, end), ...],
/// }
/// ```
///
/// # Example
///
/// ```ignore
/// span_tests! {
///     single_word: "echo" => [(0, 4)],
///     two_words: "echo hello" => [(0, 4), (5, 10)],
/// }
/// ```
macro_rules! span_tests {
    ($($name:ident: $input:expr => [$(($start:expr, $end:expr)),* $(,)?]),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let tokens = Lexer::tokenize($input).expect(concat!("failed to tokenize: ", $input));
                let expected: Vec<(usize, usize)> = vec![$(($start, $end)),*];
                let actual: Vec<_> = tokens.iter().map(|t| (t.span.start, t.span.end)).collect();
                assert_eq!(actual, expected, "input: {:?}", $input);
            }
        )*
    };
}

// Macros are exported via #[macro_use] in mod.rs
