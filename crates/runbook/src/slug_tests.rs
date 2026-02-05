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

#[test]
fn contractions_removed() {
    // "don't" → "don-t", both fragments are stop words
    assert_eq!(slugify("Don't break the login", 28), "break-login");
    // "doesn't" → "doesn-t"
    assert_eq!(slugify("Server doesn't respond", 28), "server-respond");
    // "can't" → "can-t", "can" is already a stop word
    assert_eq!(slugify("Can't load config", 28), "load-config");
    // "isn't" → "isn-t"
    assert_eq!(slugify("Value isn't valid", 28), "value-valid");
    // "hasn't" → "hasn-t"
    assert_eq!(slugify("Cache hasn't refreshed", 28), "cache-refreshed");
    // "won't" → "won-t"
    assert_eq!(slugify("Build won't pass", 28), "build-pass");
    // "shouldn't" → "shouldn-t"
    assert_eq!(slugify("This shouldn't fail", 28), "fail");
}

#[test]
fn contraction_it_s_removed() {
    // "it's" → "it-s", both "it" and "s" are stop words
    assert_eq!(slugify("It's broken", 28), "broken");
}

#[test]
fn contraction_all_stop_words() {
    // All fragments of "they're not" are stop words
    assert_eq!(slugify("they're not", 28), "");
}

#[test]
fn consecutive_duplicates_after_stop_word_removal() {
    // "peek" repeated three times should collapse to one
    assert_eq!(slugify("make end peek peek peek", 28), "make-end-peek");
}

#[test]
fn duplicates_separated_by_stop_words_collapsed() {
    // "fix the fix" → stop word "the" removed → "fix fix" → deduplicated → "fix"
    assert_eq!(slugify("fix the fix", 28), "fix");
}

#[test]
fn consecutive_duplicates_only() {
    assert_eq!(slugify("test test test", 28), "test");
}

#[test]
fn non_consecutive_duplicates_preserved() {
    // "foo bar foo" — duplicates are not consecutive, both should remain
    assert_eq!(slugify("foo bar foo", 28), "foo-bar-foo");
}

// job_display_name tests

#[test]
fn display_name_normal() {
    assert_eq!(
        job_display_name("fix-login-button", "a1b2c3d4", ""),
        "fix-login-button-a1b2c3d4"
    );
}

#[test]
fn display_name_empty_slug() {
    assert_eq!(
        job_display_name("the a an", "a1b2c3d4", ""),
        "a1b2c3d4"
    );
}

#[test]
fn display_name_with_special_chars() {
    assert_eq!(
        job_display_name("Fix the Login Button!", "abcd1234", ""),
        "fix-login-button-abcd1234"
    );
}

#[test]
fn display_name_truncation() {
    // Long input should be truncated to 28 chars before nonce
    let result = job_display_name(
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
        job_display_name("oj-queue-list-shows-queues", "90158779", "oj"),
        "queue-list-shows-queues-90158779"
    );
}

#[test]
fn display_name_no_strip_without_prefix() {
    assert_eq!(
        job_display_name("queue-list-shows-queues", "90158779", "oj"),
        "queue-list-shows-queues-90158779"
    );
}

#[test]
fn display_name_empty_namespace() {
    assert_eq!(
        job_display_name("oj-queue-list", "90158779", ""),
        "oj-queue-list-90158779"
    );
}
