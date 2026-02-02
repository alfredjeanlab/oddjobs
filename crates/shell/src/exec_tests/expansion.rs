// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for command substitution, word splitting, tilde expansion, and assignment-only.

use super::{executor, run_async};
use crate::exec::ShellExecutor;

// ---------------------------------------------------------------------------
// Command substitution
// ---------------------------------------------------------------------------

/// Tests for command substitution including nesting and variable scope isolation.
#[yare::parameterized(
    simple = { "echo $(echo inner)", None, "inner\n" },
    nested = { "echo $(echo $(echo inner))", None, "inner\n" },
    nested_with_text = { "echo prefix_$(echo mid_$(echo inner)_end)_suffix", None, "prefix_mid_inner_end_suffix\n" },
    deeply_nested = { "echo $(echo $(echo $(echo deep)))", None, "deep\n" },
    var_no_leak = { "echo $(sh -c 'VAR=inner; echo $VAR') $VAR", Some(("VAR", "outer")), "inner outer\n" },
    nested_var_no_leak = { "echo $(sh -c 'VAR=deep; echo $VAR') $VAR", Some(("VAR", "original")), "deep original\n" },
)]
fn command_substitution(script: &str, var: Option<(&str, &str)>, expected: &str) {
    run_async(async {
        let mut exec = executor();
        if let Some((k, v)) = var {
            exec = exec.variable(k, v);
        }
        let result = exec.execute_str(script).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some(expected));
    });
}

#[tokio::test]
async fn command_substitution_as_command_argument() {
    // Command substitution result is passed as argument to another command
    let result = executor()
        .execute_str("printf '%s' $(echo hello)")
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    // args includes both the format string and the substituted result
    assert_eq!(result.traces[0].args, vec!["%s", "hello"]);
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("hello"));
}

// ---------------------------------------------------------------------------
// Word splitting (IFS)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn word_split_basic_spaces() {
    let result = ShellExecutor::new()
        .variable("X", "a b c")
        .execute_str("echo $X")
        .await
        .unwrap();
    assert_eq!(result.traces[0].args, vec!["a", "b", "c"]);
}

#[tokio::test]
async fn word_split_multiple_spaces() {
    let result = ShellExecutor::new()
        .variable("X", "a   b")
        .execute_str("echo $X")
        .await
        .unwrap();
    // Multiple spaces collapse to single separator
    assert_eq!(result.traces[0].args, vec!["a", "b"]);
}

#[tokio::test]
async fn word_split_tabs_newlines() {
    let result = ShellExecutor::new()
        .variable("X", "a\tb\nc")
        .execute_str("echo $X")
        .await
        .unwrap();
    assert_eq!(result.traces[0].args, vec!["a", "b", "c"]);
}

#[tokio::test]
async fn word_split_custom_ifs() {
    let result = ShellExecutor::new()
        .variable("X", "a:b:c")
        .variable("IFS", ":")
        .execute_str("echo $X")
        .await
        .unwrap();
    assert_eq!(result.traces[0].args, vec!["a", "b", "c"]);
}

#[tokio::test]
async fn word_split_empty_ifs() {
    // Empty IFS means no splitting
    let result = ShellExecutor::new()
        .variable("X", "a b c")
        .variable("IFS", "")
        .execute_str("echo $X")
        .await
        .unwrap();
    assert_eq!(result.traces[0].args, vec!["a b c"]);
}

#[tokio::test]
async fn word_split_quoted_suppresses() {
    let result = ShellExecutor::new()
        .variable("X", "a b c")
        .execute_str(r#"echo "$X""#)
        .await
        .unwrap();
    // Double quotes suppress splitting
    assert_eq!(result.traces[0].args, vec!["a b c"]);
}

#[tokio::test]
async fn word_split_single_quoted_literal() {
    let result = executor().execute_str("echo 'a b c'").await.unwrap();
    assert_eq!(result.traces[0].args, vec!["a b c"]);
}

#[tokio::test]
async fn word_split_empty_var() {
    let result = ShellExecutor::new()
        .variable("X", "")
        .execute_str("echo $X end")
        .await
        .unwrap();
    // Empty unquoted var produces no field
    assert_eq!(result.traces[0].args, vec!["end"]);
}

#[tokio::test]
async fn word_split_quoted_empty_var() {
    let result = ShellExecutor::new()
        .variable("X", "")
        .execute_str(r#"echo "$X" end"#)
        .await
        .unwrap();
    // Quoted empty var produces one empty field
    assert_eq!(result.traces[0].args, vec!["", "end"]);
}

#[tokio::test]
async fn word_split_command_substitution() {
    let result = executor()
        .execute_str("echo $(echo 'a b c')")
        .await
        .unwrap();
    // Command substitution output is split
    assert_eq!(result.traces[0].args, vec!["a", "b", "c"]);
}

#[tokio::test]
async fn word_split_quoted_command_substitution() {
    let result = executor()
        .execute_str(r#"echo "$(echo 'a b c')""#)
        .await
        .unwrap();
    // Quoted command substitution is NOT split
    assert_eq!(result.traces[0].args, vec!["a b c"]);
}

#[tokio::test]
async fn word_split_mixed_prefix() {
    let result = ShellExecutor::new()
        .variable("X", "b c")
        .execute_str("echo a$X")
        .await
        .unwrap();
    // "a" glued to first split field
    assert_eq!(result.traces[0].args, vec!["ab", "c"]);
}

#[tokio::test]
async fn word_split_mixed_suffix() {
    let result = ShellExecutor::new()
        .variable("X", "a b")
        .execute_str("echo ${X}c")
        .await
        .unwrap();
    // "c" glued to last split field
    assert_eq!(result.traces[0].args, vec!["a", "bc"]);
}

// ---------------------------------------------------------------------------
// Tilde expansion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tilde_expands_to_home() {
    // ~ alone should expand to $HOME
    let result = executor().execute_str("echo ~").await.unwrap();
    let home = dirs::home_dir().unwrap();
    let expected = format!("{}\n", home.display());
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some(expected.as_str())
    );
}

#[tokio::test]
async fn tilde_slash_expands_to_home_subdir() {
    // ~/path should expand to $HOME/path
    let result = executor().execute_str("echo ~/Documents").await.unwrap();
    let home = dirs::home_dir().unwrap();
    let expected = format!("{}/Documents\n", home.display());
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some(expected.as_str())
    );
}

#[tokio::test]
async fn tilde_in_middle_not_expanded() {
    // Tilde in the middle of a word should not be expanded
    let result = executor().execute_str("echo foo~bar").await.unwrap();
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("foo~bar\n")
    );
}

#[tokio::test]
async fn tilde_quoted_not_expanded() {
    // Single-quoted tilde should not be expanded
    let result = executor().execute_str("echo '~'").await.unwrap();
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("~\n"));
}

#[tokio::test]
async fn tilde_double_quoted_not_expanded() {
    // Double-quoted tilde should not be expanded
    let result = executor().execute_str(r#"echo "~""#).await.unwrap();
    assert_eq!(result.traces[0].stdout_snippet.as_deref(), Some("~\n"));
}

#[tokio::test]
async fn tilde_with_variable_suffix() {
    // ~/foo$VAR should expand ~ and then append variable
    let result = ShellExecutor::new()
        .variable("DIR", "bar")
        .execute_str("echo ~/$DIR")
        .await
        .unwrap();
    let home = dirs::home_dir().unwrap();
    let expected = format!("{}/bar\n", home.display());
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some(expected.as_str())
    );
}

#[tokio::test]
#[cfg(unix)]
async fn tilde_user_expands_to_user_home() {
    // ~root should expand to root's home directory (usually /root or /var/root)
    // We'll test with the current user instead to be more reliable
    let current_user = std::env::var("USER").unwrap_or_else(|_| "root".to_string());
    let script = format!("echo ~{current_user}");
    let result = executor().execute_str(&script).await.unwrap();

    // The output should be the user's home directory (not ~user literal)
    let output = result.traces[0].stdout_snippet.as_deref().unwrap();
    assert!(
        !output.starts_with('~'),
        "tilde should be expanded: {output}"
    );
    // Should be a valid path
    assert!(
        output.starts_with('/'),
        "should be an absolute path: {output}"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn tilde_user_path_expands() {
    // ~user/path should expand to user's home + path
    let current_user = std::env::var("USER").unwrap_or_else(|_| "root".to_string());
    let script = format!("echo ~{current_user}/Documents");
    let result = executor().execute_str(&script).await.unwrap();

    let output = result.traces[0].stdout_snippet.as_deref().unwrap();
    assert!(
        !output.starts_with('~'),
        "tilde should be expanded: {output}"
    );
    assert!(
        output.contains("/Documents"),
        "should contain path suffix: {output}"
    );
}

#[tokio::test]
async fn tilde_unknown_user_not_expanded() {
    // ~nonexistent_user_xyz should remain unexpanded
    let result = executor()
        .execute_str("echo ~nonexistent_user_xyz_12345")
        .await
        .unwrap();
    assert_eq!(
        result.traces[0].stdout_snippet.as_deref(),
        Some("~nonexistent_user_xyz_12345\n")
    );
}

// ---------------------------------------------------------------------------
// Assignment-only commands (VAR=value without a command)
// ---------------------------------------------------------------------------

/// Tests for assignment-only commands (VAR=value without a command).
#[yare::parameterized(
    single = { "FOO=bar; echo $FOO", "bar\n" },
    multiple = { "A=1 B=2; echo $A $B", "1 2\n" },
)]
fn assignment_only_sets_variable(script: &str, expected: &str) {
    run_async(async {
        let result = executor().execute_str(script).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.traces.len(), 2);
        assert_eq!(result.traces[0].command, ""); // assignment-only has empty command
        assert_eq!(result.traces[1].stdout_snippet.as_deref(), Some(expected));
    });
}

#[tokio::test]
async fn assignment_only_returns_zero() {
    let result = executor().execute_str("FOO=bar").await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.traces.len(), 1);
    assert_eq!(result.traces[0].command, "");
}

// ---------------------------------------------------------------------------
// Glob expansion with word splitting
// ---------------------------------------------------------------------------

#[tokio::test]
async fn glob_unquoted_expands() {
    // Create temp directory with test files
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a.txt")).unwrap();
    std::fs::File::create(dir.path().join("b.txt")).unwrap();
    std::fs::File::create(dir.path().join("c.rs")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .execute_str("echo *.txt")
        .await
        .unwrap();

    // Unquoted glob should expand to matching files
    assert_eq!(result.traces[0].args, vec!["a.txt", "b.txt"]);
}

#[tokio::test]
async fn glob_quoted_literal() {
    // Create temp directory with test files
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .execute_str(r#"echo "*.txt""#)
        .await
        .unwrap();

    // Quoted glob should NOT expand - remains literal
    assert_eq!(result.traces[0].args, vec!["*.txt"]);
}

#[tokio::test]
async fn glob_single_quoted_literal() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .execute_str("echo '*.txt'")
        .await
        .unwrap();

    // Single-quoted glob should NOT expand
    assert_eq!(result.traces[0].args, vec!["*.txt"]);
}

#[tokio::test]
async fn glob_from_variable_does_not_expand() {
    // Variable containing glob pattern should NOT expand
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a.txt")).unwrap();
    std::fs::File::create(dir.path().join("b.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .variable("PATTERN", "*.txt")
        .execute_str("echo $PATTERN")
        .await
        .unwrap();

    // Variable expansion result should NOT be glob-expanded
    assert_eq!(result.traces[0].args, vec!["*.txt"]);
}

#[tokio::test]
async fn glob_no_match_returns_literal() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .execute_str("echo *.xyz")
        .await
        .unwrap();

    // POSIX: no matches returns literal pattern
    assert_eq!(result.traces[0].args, vec!["*.xyz"]);
}

#[tokio::test]
async fn glob_with_word_split() {
    // Word splitting + glob expansion together
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a.txt")).unwrap();
    std::fs::File::create(dir.path().join("b.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .variable("PREFIX", "a b")
        .execute_str("echo $PREFIX *.txt")
        .await
        .unwrap();

    // $PREFIX splits to ["a", "b"], *.txt globs to ["a.txt", "b.txt"]
    assert_eq!(result.traces[0].args, vec!["a", "b", "a.txt", "b.txt"]);
}

#[tokio::test]
async fn glob_mixed_literal_and_variable() {
    // Literal glob chars mixed with variable (only literal should glob)
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("prefix_a.txt")).unwrap();
    std::fs::File::create(dir.path().join("prefix_b.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .variable("PRE", "prefix")
        .execute_str("echo ${PRE}_*.txt")
        .await
        .unwrap();

    // Variable part is not glob-eligible, but literal * is
    assert_eq!(result.traces[0].args, vec!["prefix_a.txt", "prefix_b.txt"]);
}

#[tokio::test]
async fn glob_question_mark_expands() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a1.txt")).unwrap();
    std::fs::File::create(dir.path().join("a2.txt")).unwrap();
    std::fs::File::create(dir.path().join("abc.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .execute_str("echo a?.txt")
        .await
        .unwrap();

    // ? matches single character
    assert_eq!(result.traces[0].args, vec!["a1.txt", "a2.txt"]);
}

#[tokio::test]
async fn glob_character_class_expands() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a1.txt")).unwrap();
    std::fs::File::create(dir.path().join("a2.txt")).unwrap();
    std::fs::File::create(dir.path().join("a3.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .execute_str("echo a[12].txt")
        .await
        .unwrap();

    // [12] matches 1 or 2
    assert_eq!(result.traces[0].args, vec!["a1.txt", "a2.txt"]);
}

// ---------------------------------------------------------------------------
// Escaped glob metacharacters (backslash suppresses glob expansion)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn glob_escaped_asterisk_not_expanded() {
    // \* should produce literal *, not glob-expand
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a.txt")).unwrap();
    std::fs::File::create(dir.path().join("b.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .execute_str(r"echo \*.txt")
        .await
        .unwrap();

    // Escaped * should NOT expand - remains literal
    assert_eq!(result.traces[0].args, vec!["*.txt"]);
}

#[tokio::test]
async fn glob_escaped_question_not_expanded() {
    // \? should produce literal ?, not glob-expand
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a1.txt")).unwrap();
    std::fs::File::create(dir.path().join("a2.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .execute_str(r"echo a\?.txt")
        .await
        .unwrap();

    // Escaped ? should NOT expand - remains literal
    assert_eq!(result.traces[0].args, vec!["a?.txt"]);
}

#[tokio::test]
async fn glob_escaped_bracket_not_expanded() {
    // \[ should produce literal [, not start a character class
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a1.txt")).unwrap();
    std::fs::File::create(dir.path().join("a2.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .execute_str(r"echo a\[12].txt")
        .await
        .unwrap();

    // Escaped [ should NOT expand - remains literal
    assert_eq!(result.traces[0].args, vec!["a[12].txt"]);
}

#[tokio::test]
async fn glob_double_backslash_then_asterisk() {
    // \\ at the lexer level: first backslash escapes the second, producing \
    // Then * is unescaped, remaining as * in the token.
    // Lexer output: \*
    // At expansion: \* is processed, backslash escapes *, producing literal *
    // So \\* expands to just * (no glob).
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::File::create(dir.path().join("a.txt")).unwrap();

    let result = ShellExecutor::new()
        .cwd(dir.path())
        .execute_str(r"echo \\*.txt")
        .await
        .unwrap();

    // \\*.txt → lexer \*.txt → expansion *.txt (literal, no glob)
    assert_eq!(result.traces[0].args, vec!["*.txt"]);
}
