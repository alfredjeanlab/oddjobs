// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Unit tests for glob expansion.

use std::fs::{self, File};
use std::path::Path;

use tempfile::TempDir;

use super::*;

// =============================================================================
// GlobEligibility tests
// =============================================================================

#[test]
fn glob_eligibility_empty() {
    let e = GlobEligibility::new();
    assert!(!e.has_glob_pattern());
    assert_eq!(e.text, "");
}

#[test]
fn glob_eligibility_eligible_asterisk() {
    let mut e = GlobEligibility::new();
    e.push_eligible("*.txt");
    assert!(e.has_glob_pattern());
    assert_eq!(e.text, "*.txt");
}

#[test]
fn glob_eligibility_ineligible_asterisk() {
    let mut e = GlobEligibility::new();
    e.push_ineligible("*.txt");
    assert!(!e.has_glob_pattern());
    assert_eq!(e.text, "*.txt");
}

#[test]
fn glob_eligibility_mixed() {
    let mut e = GlobEligibility::new();
    e.push_ineligible("prefix"); // from quoted
    e.push_eligible("*.txt"); // from unquoted
    assert!(e.has_glob_pattern());
    assert_eq!(e.text, "prefix*.txt");
}

#[test]
fn glob_eligibility_mixed_no_pattern() {
    let mut e = GlobEligibility::new();
    e.push_ineligible("*"); // asterisk from quoted - not eligible
    e.push_eligible("file.txt"); // no metacharacters
    assert!(!e.has_glob_pattern());
    assert_eq!(e.text, "*file.txt");
}

// =============================================================================
// expand_glob_pattern tests
// =============================================================================

fn create_test_files(dir: &Path, names: &[&str]) {
    for name in names {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            if parent != dir {
                fs::create_dir_all(parent).unwrap();
            }
        }
        File::create(path).unwrap();
    }
}

#[test]
fn expand_glob_asterisk_txt() {
    let dir = TempDir::new().unwrap();
    create_test_files(dir.path(), &["a.txt", "b.txt", "c.rs"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern("*.txt", dir.path(), &config, Span::default()).unwrap();

    assert_eq!(result, vec!["a.txt", "b.txt"]);
}

#[test]
fn expand_glob_asterisk_rs() {
    let dir = TempDir::new().unwrap();
    create_test_files(dir.path(), &["a.txt", "b.txt", "c.rs"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern("*.rs", dir.path(), &config, Span::default()).unwrap();

    assert_eq!(result, vec!["c.rs"]);
}

#[test]
fn expand_glob_asterisk_all() {
    let dir = TempDir::new().unwrap();
    create_test_files(dir.path(), &["a.txt", "b.txt", "c.rs"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern("*", dir.path(), &config, Span::default()).unwrap();

    // Should match all non-hidden files
    assert_eq!(result, vec!["a.txt", "b.txt", "c.rs"]);
}

#[test]
fn expand_glob_question_mark() {
    let dir = TempDir::new().unwrap();
    create_test_files(dir.path(), &["a1.txt", "a2.txt", "abc.txt"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern("a?.txt", dir.path(), &config, Span::default()).unwrap();

    // ? matches any single character, so a1.txt and a2.txt match
    // abc.txt does not match because "bc" is two characters
    assert_eq!(result, vec!["a1.txt", "a2.txt"]);
}

#[test]
fn expand_glob_character_class() {
    let dir = TempDir::new().unwrap();
    create_test_files(dir.path(), &["a1.txt", "a2.txt", "a3.txt"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern("a[12].txt", dir.path(), &config, Span::default()).unwrap();

    assert_eq!(result, vec!["a1.txt", "a2.txt"]);
}

#[test]
fn expand_glob_negated_character_class() {
    let dir = TempDir::new().unwrap();
    create_test_files(dir.path(), &["a1.txt", "a2.txt", "a3.txt"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern("a[!1].txt", dir.path(), &config, Span::default()).unwrap();

    assert_eq!(result, vec!["a2.txt", "a3.txt"]);
}

#[test]
fn expand_glob_no_match_posix() {
    let dir = TempDir::new().unwrap();
    create_test_files(dir.path(), &["a.txt"]);

    let config = GlobConfig { nullglob: false };
    let result = expand_glob_pattern("*.xyz", dir.path(), &config, Span::default()).unwrap();

    // POSIX: return literal pattern when no matches
    assert_eq!(result, vec!["*.xyz"]);
}

#[test]
fn expand_glob_no_match_nullglob() {
    let dir = TempDir::new().unwrap();
    create_test_files(dir.path(), &["a.txt"]);

    let config = GlobConfig { nullglob: true };
    let result = expand_glob_pattern("*.xyz", dir.path(), &config, Span::default()).unwrap();

    // nullglob: return empty vec when no matches
    assert!(result.is_empty());
}

#[test]
fn expand_glob_sorted_results() {
    let dir = TempDir::new().unwrap();
    // Create in reverse order to verify sorting
    create_test_files(dir.path(), &["z.txt", "a.txt", "m.txt"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern("*.txt", dir.path(), &config, Span::default()).unwrap();

    assert_eq!(result, vec!["a.txt", "m.txt", "z.txt"]);
}

#[test]
fn expand_glob_subdirectory() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("subdir")).unwrap();
    create_test_files(dir.path(), &["subdir/file1.txt", "subdir/file2.txt"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern("subdir/*.txt", dir.path(), &config, Span::default()).unwrap();

    assert_eq!(result, vec!["subdir/file1.txt", "subdir/file2.txt"]);
}

#[test]
fn expand_glob_recursive() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("a/b")).unwrap();
    create_test_files(dir.path(), &["root.txt", "a/mid.txt", "a/b/deep.txt"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern("**/*.txt", dir.path(), &config, Span::default()).unwrap();

    // The glob crate supports ** for recursive matching
    assert!(result.contains(&"a/mid.txt".to_string()));
    assert!(result.contains(&"a/b/deep.txt".to_string()));
}

#[test]
fn expand_glob_hidden_files_not_matched() {
    let dir = TempDir::new().unwrap();
    create_test_files(dir.path(), &["visible.txt", ".hidden.txt"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern("*.txt", dir.path(), &config, Span::default()).unwrap();

    // By default, * should NOT match hidden files (starting with .)
    assert_eq!(result, vec!["visible.txt"]);
}

#[test]
fn expand_glob_hidden_files_explicit() {
    let dir = TempDir::new().unwrap();
    create_test_files(dir.path(), &[".hidden.txt", ".other.txt"]);

    let config = GlobConfig::default();
    let result = expand_glob_pattern(".*.txt", dir.path(), &config, Span::default()).unwrap();

    // Pattern starting with . should match hidden files with .txt extension
    assert_eq!(result, vec![".hidden.txt", ".other.txt"]);
}

#[test]
fn expand_glob_invalid_pattern() {
    let dir = TempDir::new().unwrap();

    let config = GlobConfig::default();
    let result = expand_glob_pattern("[abc", dir.path(), &config, Span::default());

    // Unclosed bracket should produce an error
    assert!(result.is_err());
    if let Err(ExecError::GlobPattern { pattern, .. }) = result {
        assert_eq!(pattern, "[abc");
    } else {
        panic!("Expected GlobPattern error");
    }
}
