// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[yare::parameterized(
    no_special_chars       = { "hello world",                                     "hello world" },
    escapes_backslash      = { r"path\to\file",                                   r"path\\to\\file" },
    escapes_dollar_sign    = { "$HOME",                                            "\\$HOME" },
    escapes_backtick       = { "Write to `file.txt`",                             "Write to \\`file.txt\\`" },
    escapes_double_quote   = { r#"say "hello""#,                                  r#"say \"hello\""# },
    escapes_all_special    = { r#"$VAR `cmd` "quote" \slash"#,                    r#"\$VAR \`cmd\` \"quote\" \\slash"# },
    empty_string           = { "",                                                 "" },
    preserves_single_quote = { "it's a test",                                     "it's a test" },
    preserves_whitespace   = { "Normal text with newlines\nand tabs\t",           "Normal text with newlines\nand tabs\t" },
)]
fn escape_for_shell_cases(input: &str, expected: &str) {
    assert_eq!(escape_for_shell(input), expected);
}

#[yare::parameterized(
    escapes_special_chars = { r#"git commit -m "${title}""#,  &[("title", r#"fix: handle "$HOME" path"#)],  r#"git commit -m "fix: handle \"\$HOME\" path""# },
    escapes_backticks     = { r#"git commit -m "${title}""#,  &[("title", "fix: update `config.rs`")],      r#"git commit -m "fix: update \`config.rs\`""# },
    preserves_single_quotes = { r#"echo "${msg}""#,           &[("msg", "it's a test")],                    r#"echo "it's a test""# },
    unknown_left_alone    = { "echo '${unknown}'",            &[],                                          "echo '${unknown}'" },
)]
fn interpolate_shell_cases(template: &str, var_pairs: &[(&str, &str)], expected: &str) {
    assert_eq!(interpolate_shell(template, &vars(var_pairs)), expected);
}

#[test]
fn interpolate_plain_does_not_escape() {
    let v = vars(&[("msg", r#"$HOME `pwd` "hello""#)]);
    assert_eq!(interpolate("${msg}", &v), r#"$HOME `pwd` "hello""#);
}

#[test]
fn interpolate_shell_realistic_submit_step() {
    let v = vars(&[
        ("local.title", "fix: handle `$PATH` and \"quotes\""),
        ("local.branch", "fix/bug-123"),
    ]);
    let template = r#"git commit -m "${local.title}" && git push origin "${local.branch}""#;
    assert_eq!(
        interpolate_shell(template, &v),
        r#"git commit -m "fix: handle \`\$PATH\` and \"quotes\"" && git push origin "fix/bug-123""#
    );
}

#[yare::parameterized(
    simple          = { "Hello ${name}!",             &[("name", "test")],         "Hello test!" },
    multiple        = { "${a} + ${b} = ${a}${b}",     &[("a", "1"), ("b", "2")],   "1 + 2 = 12" },
    unknown         = { "Hello ${unknown}!",          &[],                          "Hello ${unknown}!" },
    no_vars         = { "No variables here",          &[],                          "No variables here" },
    empty_braces    = { "${}",                        &[],                          "${}" },
    incomplete      = { "${",                         &[],                          "${" },
)]
fn interpolate_basic(template: &str, var_pairs: &[(&str, &str)], expected: &str) {
    assert_eq!(interpolate(template, &vars(var_pairs)), expected);
}

#[test]
fn interpolate_env_var_with_default_uses_env() {
    // Set an env var for this test
    std::env::set_var("TEMPLATE_TEST_VAR", "from_env");
    let vars: HashMap<String, String> = HashMap::new();
    assert_eq!(interpolate("${TEMPLATE_TEST_VAR:-default}", &vars), "from_env");
    std::env::remove_var("TEMPLATE_TEST_VAR");
}

#[test]
fn interpolate_env_var_with_default_uses_default() {
    // Ensure env var is not set
    std::env::remove_var("TEMPLATE_UNSET_VAR");
    let vars: HashMap<String, String> = HashMap::new();
    assert_eq!(interpolate("${TEMPLATE_UNSET_VAR:-fallback}", &vars), "fallback");
}

#[test]
fn interpolate_env_and_template_vars() {
    std::env::set_var("TEMPLATE_CMD_VAR", "custom_cmd");
    let vars: HashMap<String, String> =
        [("name".to_string(), "test".to_string())].into_iter().collect();
    assert_eq!(
        interpolate("${TEMPLATE_CMD_VAR:-default} --name ${name}", &vars),
        "custom_cmd --name test"
    );
    std::env::remove_var("TEMPLATE_CMD_VAR");
}

#[yare::parameterized(
    dotted_key    = { "Feature: ${input.name}, Task: ${input.prompt}", &[("input.name", "my-feature"), ("input.prompt", "Add tests")], "Feature: my-feature, Task: Add tests" },
    dotted_hyphen = { "Testing ${input.feature-name}",                 &[("input.feature-name", "auth")],                              "Testing auth" },
    mixed         = { "Command: ${prompt}, Input: ${input.prompt}",    &[("prompt", "rendered prompt text"), ("input.prompt", "user input")], "Command: rendered prompt text, Input: user input" },
)]
fn interpolate_dotted(template: &str, var_pairs: &[(&str, &str)], expected: &str) {
    assert_eq!(interpolate(template, &vars(var_pairs)), expected);
}

#[yare::parameterized(
    offset_and_length = { "${name:0:5}",                  &[("name", "hello world")],                                  "hello" },
    offset_only       = { "${name:6}",                    &[("name", "hello world")],                                  "world" },
    no_slice          = { "${name}",                      &[("name", "hello world")],                                  "hello world" },
    unknown_var       = { "${unknown:0:5}",               &[],                                                         "${unknown:0:5}" },
    beyond_length     = { "${name:0:100}",                &[("name", "short")],                                        "short" },
    dotted_key        = { "feat: ${var.instructions:0:20}", &[("var.instructions", "Add feature for handling long descriptions")], "feat: Add feature for hand" },
    unicode           = { "${name:0:5}",                  &[("name", "héllo wörld")],                                  "héllo" },
)]
fn interpolate_substring(template: &str, var_pairs: &[(&str, &str)], expected: &str) {
    assert_eq!(interpolate(template, &vars(var_pairs)), expected);
}

#[test]
fn interpolate_substring_shell_escaping_after_truncation() {
    let v = vars(&[("msg", "safe $dollar `tick`")]);
    assert_eq!(interpolate_shell("${msg:0:9}", &v), "safe \\$dol");
}
