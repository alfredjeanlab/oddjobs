// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn basic_slugify() {
    assert_eq!(slugify("Hello World", 28), "hello-world");
}

#[test]
fn stop_words_removed() {
    assert_eq!(slugify("Fix the login button", 28), "fix-login-button");
}

#[test]
fn non_alphanum_replaced() {
    assert_eq!(slugify("fix: login_button!", 28), "fix-login-button");
}

#[test]
fn multiple_hyphens_collapsed() {
    assert_eq!(slugify("foo---bar", 28), "foo-bar");
}

#[test]
fn truncation_at_word_boundary() {
    // "implement-user-authentication-system" is 36 chars, truncated at last hyphen before 28
    let result = slugify("Implement User Authentication System", 28);
    assert!(result.len() <= 28);
    assert!(!result.ends_with('-'));
    assert_eq!(result, "implement-user");
}

#[test]
fn truncation_single_long_word() {
    // A single word longer than max_len gets hard-truncated
    let result = slugify("abcdefghijklmnopqrstuvwxyz12345", 28);
    assert_eq!(result, "abcdefghijklmnopqrstuvwxyz12");
}

#[test]
fn empty_after_stop_word_removal() {
    assert_eq!(slugify("the a an is are", 28), "");
}

#[test]
fn already_clean_slug() {
    assert_eq!(slugify("fix-login-button", 28), "fix-login-button");
}

#[test]
fn unicode_chars_replaced() {
    assert_eq!(slugify("café résumé", 28), "caf-r-sum");
}

#[test]
fn leading_trailing_hyphens_trimmed() {
    assert_eq!(slugify("--hello--", 28), "hello");
}

#[test]
fn single_word() {
    assert_eq!(slugify("deploy", 28), "deploy");
}

#[test]
fn all_special_chars() {
    assert_eq!(slugify("!!@@##$$", 28), "");
}

#[test]
fn exact_max_len() {
    // "abcdefghijklmnopqrstuvwxyz12" is exactly 28 chars
    assert_eq!(
        slugify("abcdefghijklmnopqrstuvwxyz12", 28),
        "abcdefghijklmnopqrstuvwxyz12"
    );
}

#[test]
fn truncation_trims_trailing_hyphen() {
    // Construct input that will produce a slug with a hyphen near position 28
    let result = slugify("abcdefghijklmnopqrstuvwxyz1 xyz", 28);
    assert!(!result.ends_with('-'));
    assert!(result.len() <= 28);
}

// pipeline_display_name tests

#[test]
fn display_name_normal() {
    assert_eq!(
        pipeline_display_name("fix-login-button", "a1b2c3d4", ""),
        "fix-login-button-a1b2c3d4"
    );
}

#[test]
fn display_name_empty_slug() {
    assert_eq!(
        pipeline_display_name("the a an", "a1b2c3d4", ""),
        "a1b2c3d4"
    );
}

#[test]
fn display_name_with_special_chars() {
    assert_eq!(
        pipeline_display_name("Fix the Login Button!", "abcd1234", ""),
        "fix-login-button-abcd1234"
    );
}

#[test]
fn display_name_truncation() {
    // Long input should be truncated to 28 chars before nonce
    let result = pipeline_display_name(
        "implement user authentication system for the app",
        "12345678",
        "",
    );
    let parts: Vec<&str> = result.rsplitn(2, '-').collect();
    assert_eq!(parts[0], "12345678");
    let slug_part = parts[1];
    assert!(slug_part.len() <= 28);
}

#[test]
fn display_name_strips_namespace_prefix() {
    assert_eq!(
        pipeline_display_name("oj-queue-list-shows-queues", "90158779", "oj"),
        "queue-list-shows-queues-90158779"
    );
}

#[test]
fn display_name_no_strip_without_prefix() {
    assert_eq!(
        pipeline_display_name("queue-list-shows-queues", "90158779", "oj"),
        "queue-list-shows-queues-90158779"
    );
}

#[test]
fn display_name_empty_namespace() {
    assert_eq!(
        pipeline_display_name("oj-queue-list", "90158779", ""),
        "oj-queue-list-90158779"
    );
}
