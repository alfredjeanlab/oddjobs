// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn basic_slugify() {
    assert_eq!(slugify("Hello World", 24), "hello-world");
}

#[test]
fn stop_words_removed() {
    assert_eq!(slugify("Fix the login button", 24), "fix-login-button");
}

#[test]
fn non_alphanum_replaced() {
    assert_eq!(slugify("fix: login_button!", 24), "fix-login-button");
}

#[test]
fn multiple_hyphens_collapsed() {
    assert_eq!(slugify("foo---bar", 24), "foo-bar");
}

#[test]
fn truncation_at_max_len() {
    // "implement-user-authenticat" is 26 chars, should truncate to 24
    let result = slugify("Implement User Authentication System", 24);
    assert!(result.len() <= 24);
    assert!(!result.ends_with('-'));
    assert_eq!(
        result,
        "implement-user-authenticat"
            .get(..24)
            .unwrap()
            .trim_end_matches('-')
    );
}

#[test]
fn empty_after_stop_word_removal() {
    assert_eq!(slugify("the a an is are", 24), "");
}

#[test]
fn already_clean_slug() {
    assert_eq!(slugify("fix-login-button", 24), "fix-login-button");
}

#[test]
fn unicode_chars_replaced() {
    assert_eq!(slugify("café résumé", 24), "caf-r-sum");
}

#[test]
fn leading_trailing_hyphens_trimmed() {
    assert_eq!(slugify("--hello--", 24), "hello");
}

#[test]
fn single_word() {
    assert_eq!(slugify("deploy", 24), "deploy");
}

#[test]
fn all_special_chars() {
    assert_eq!(slugify("!!@@##$$", 24), "");
}

#[test]
fn exact_max_len() {
    // "abcdefghijklmnopqrstuvwx" is exactly 24 chars
    assert_eq!(
        slugify("abcdefghijklmnopqrstuvwx", 24),
        "abcdefghijklmnopqrstuvwx"
    );
}

#[test]
fn truncation_trims_trailing_hyphen() {
    // Construct input that will produce a slug with a hyphen at position 24
    let result = slugify("abcdefghijklmnopqrstuvw xyz", 24);
    assert!(!result.ends_with('-'));
    assert!(result.len() <= 24);
}

// pipeline_display_name tests

#[test]
fn display_name_normal() {
    assert_eq!(
        pipeline_display_name("fix-login-button", "a1b2c3d4"),
        "fix-login-button-a1b2c3d4"
    );
}

#[test]
fn display_name_empty_slug() {
    assert_eq!(pipeline_display_name("the a an", "a1b2c3d4"), "a1b2c3d4");
}

#[test]
fn display_name_with_special_chars() {
    assert_eq!(
        pipeline_display_name("Fix the Login Button!", "abcd1234"),
        "fix-login-button-abcd1234"
    );
}

#[test]
fn display_name_truncation() {
    // Long input should be truncated to 24 chars before nonce
    let result = pipeline_display_name(
        "implement user authentication system for the app",
        "12345678",
    );
    let parts: Vec<&str> = result.rsplitn(2, '-').collect();
    assert_eq!(parts[0], "12345678");
    let slug_part = parts[1];
    assert!(slug_part.len() <= 24);
}
