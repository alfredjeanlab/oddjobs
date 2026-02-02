// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

// =============================================================================
// escape_for_shell tests
// =============================================================================

#[test]
fn escape_for_shell_no_quotes() {
    assert_eq!(escape_for_shell("hello world"), "hello world");
}

#[test]
fn escape_for_shell_single_quote() {
    assert_eq!(escape_for_shell("it's a test"), "it'\\''s a test");
}

#[test]
fn escape_for_shell_multiple_single_quotes() {
    assert_eq!(escape_for_shell("it's Bob's"), "it'\\''s Bob'\\''s");
}

#[test]
fn escape_for_shell_empty_string() {
    assert_eq!(escape_for_shell(""), "");
}

#[test]
fn escape_for_shell_preserves_double_quotes() {
    // Double quotes don't need escaping for single-quote context
    assert_eq!(escape_for_shell(r#"say "hello""#), r#"say "hello""#);
}

#[test]
fn escape_for_shell_preserves_special_chars() {
    // Dollar signs and backticks are literal inside single quotes
    assert_eq!(escape_for_shell("$HOME `pwd`"), "$HOME `pwd`");
}

// =============================================================================
// interpolate_shell tests
// =============================================================================

#[test]
fn interpolate_shell_escapes_single_quotes() {
    let vars: HashMap<String, String> = [("msg".to_string(), "it's a test".to_string())]
        .into_iter()
        .collect();
    assert_eq!(
        interpolate_shell("echo '${msg}'", &vars),
        "echo 'it'\\''s a test'"
    );
}

#[test]
fn interpolate_shell_preserves_double_quotes_and_specials() {
    let vars: HashMap<String, String> = [("msg".to_string(), r#"say "hello" $HOME"#.to_string())]
        .into_iter()
        .collect();
    // Double quotes and $ are safe inside single-quoted shell context
    assert_eq!(
        interpolate_shell("echo '${msg}'", &vars),
        r#"echo 'say "hello" $HOME'"#
    );
}

#[test]
fn interpolate_shell_unknown_left_alone() {
    let vars: HashMap<String, String> = HashMap::new();
    assert_eq!(
        interpolate_shell("echo '${unknown}'", &vars),
        "echo '${unknown}'"
    );
}

#[test]
fn interpolate_plain_does_not_escape() {
    let vars: HashMap<String, String> = [("msg".to_string(), "it's a test".to_string())]
        .into_iter()
        .collect();
    // Regular interpolate should NOT escape
    assert_eq!(interpolate("${msg}", &vars), "it's a test");
}

// =============================================================================
// interpolate tests
// =============================================================================

#[test]
fn interpolate_simple() {
    let vars: HashMap<String, String> = [("name".to_string(), "test".to_string())]
        .into_iter()
        .collect();
    assert_eq!(interpolate("Hello ${name}!", &vars), "Hello test!");
}

#[test]
fn interpolate_multiple() {
    let vars: HashMap<String, String> = [
        ("a".to_string(), "1".to_string()),
        ("b".to_string(), "2".to_string()),
    ]
    .into_iter()
    .collect();
    assert_eq!(interpolate("${a} + ${b} = ${a}${b}", &vars), "1 + 2 = 12");
}

#[test]
fn interpolate_unknown_left_alone() {
    let vars: HashMap<String, String> = HashMap::new();
    assert_eq!(interpolate("Hello ${unknown}!", &vars), "Hello ${unknown}!");
}

#[test]
fn interpolate_no_vars() {
    let vars: HashMap<String, String> = HashMap::new();
    assert_eq!(interpolate("No variables here", &vars), "No variables here");
}

#[test]
fn interpolate_empty_braces_not_matched() {
    let vars: HashMap<String, String> = HashMap::new();
    // Empty ${} should not match the template var pattern and pass through unchanged
    assert_eq!(interpolate("${}", &vars), "${}");
    // Incomplete ${ should also pass through unchanged
    assert_eq!(interpolate("${", &vars), "${");
}

#[test]
fn interpolate_env_var_with_default_uses_env() {
    // Set an env var for this test
    std::env::set_var("TEMPLATE_TEST_VAR", "from_env");
    let vars: HashMap<String, String> = HashMap::new();
    assert_eq!(
        interpolate("${TEMPLATE_TEST_VAR:-default}", &vars),
        "from_env"
    );
    std::env::remove_var("TEMPLATE_TEST_VAR");
}

#[test]
fn interpolate_env_var_with_default_uses_default() {
    // Ensure env var is not set
    std::env::remove_var("TEMPLATE_UNSET_VAR");
    let vars: HashMap<String, String> = HashMap::new();
    assert_eq!(
        interpolate("${TEMPLATE_UNSET_VAR:-fallback}", &vars),
        "fallback"
    );
}

#[test]
fn interpolate_env_and_template_vars() {
    std::env::set_var("TEMPLATE_CMD_VAR", "custom_cmd");
    let vars: HashMap<String, String> = [("name".to_string(), "test".to_string())]
        .into_iter()
        .collect();
    assert_eq!(
        interpolate("${TEMPLATE_CMD_VAR:-default} --name ${name}", &vars),
        "custom_cmd --name test"
    );
    std::env::remove_var("TEMPLATE_CMD_VAR");
}

#[test]
fn interpolate_dotted_key() {
    let vars: HashMap<String, String> = [
        ("input.name".to_string(), "my-feature".to_string()),
        ("input.prompt".to_string(), "Add tests".to_string()),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        interpolate("Feature: ${input.name}, Task: ${input.prompt}", &vars),
        "Feature: my-feature, Task: Add tests"
    );
}

#[test]
fn interpolate_dotted_key_with_hyphen() {
    let vars: HashMap<String, String> = [("input.feature-name".to_string(), "auth".to_string())]
        .into_iter()
        .collect();
    assert_eq!(
        interpolate("Testing ${input.feature-name}", &vars),
        "Testing auth"
    );
}

#[test]
fn interpolate_mixed_simple_and_dotted() {
    let vars: HashMap<String, String> = [
        ("prompt".to_string(), "rendered prompt text".to_string()),
        ("input.prompt".to_string(), "user input".to_string()),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        interpolate("Command: ${prompt}, Input: ${input.prompt}", &vars),
        "Command: rendered prompt text, Input: user input"
    );
}
