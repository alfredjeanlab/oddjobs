// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Tests for env file parsing and loading

use super::*;
use std::collections::BTreeMap;
use tempfile::TempDir;

#[test]
fn parse_env_empty_input() {
    assert!(parse_env("").is_empty());
}

#[test]
fn parse_env_comments_and_blank_lines() {
    let content = "# comment\n\n# another comment\n";
    assert!(parse_env(content).is_empty());
}

#[test]
fn parse_env_valid_pairs() {
    let content = "FOO=bar\nBAZ=qux\n";
    let map = parse_env(content);
    assert_eq!(map.get("FOO"), Some(&"bar".to_string()));
    assert_eq!(map.get("BAZ"), Some(&"qux".to_string()));
}

#[test]
fn parse_env_value_with_equals() {
    let content = "API_KEY=abc=def=123\n";
    let map = parse_env(content);
    assert_eq!(map.get("API_KEY"), Some(&"abc=def=123".to_string()));
}

#[test]
fn parse_env_value_with_spaces() {
    let content = "MSG=hello world\n";
    let map = parse_env(content);
    assert_eq!(map.get("MSG"), Some(&"hello world".to_string()));
}

#[test]
fn parse_env_trims_key_whitespace() {
    let content = "  FOO  =bar\n";
    let map = parse_env(content);
    assert_eq!(map.get("FOO"), Some(&"bar".to_string()));
}

#[test]
fn parse_env_skips_lines_without_equals() {
    let content = "FOO=bar\nINVALID_LINE\nBAZ=qux\n";
    let map = parse_env(content);
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("FOO"), Some(&"bar".to_string()));
    assert_eq!(map.get("BAZ"), Some(&"qux".to_string()));
}

#[test]
fn parse_env_empty_value() {
    let content = "EMPTY=\n";
    let map = parse_env(content);
    assert_eq!(map.get("EMPTY"), Some(&"".to_string()));
}

#[test]
fn read_env_file_missing_returns_empty() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nonexistent");
    let result = read_env_file(&path).unwrap();
    assert!(result.is_empty());
}

#[test]
fn write_and_read_env_file_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("env");

    let mut vars = BTreeMap::new();
    vars.insert("A".to_string(), "1".to_string());
    vars.insert("B".to_string(), "two".to_string());
    vars.insert("C".to_string(), "x=y=z".to_string());

    write_env_file(&path, &vars).unwrap();
    let loaded = read_env_file(&path).unwrap();
    assert_eq!(loaded, vars);
}

#[test]
fn write_env_file_empty_map_removes_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("env");

    // Create the file first
    let mut vars = BTreeMap::new();
    vars.insert("KEY".to_string(), "val".to_string());
    write_env_file(&path, &vars).unwrap();
    assert!(path.exists());

    // Write empty map — should remove the file
    write_env_file(&path, &BTreeMap::new()).unwrap();
    assert!(!path.exists());
}

#[test]
fn write_env_file_empty_map_noop_if_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nonexistent");
    // Should not error when removing a file that doesn't exist
    write_env_file(&path, &BTreeMap::new()).unwrap();
}

#[test]
fn write_env_file_creates_parent_dirs() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("dir").join("env");

    let mut vars = BTreeMap::new();
    vars.insert("KEY".to_string(), "val".to_string());
    write_env_file(&path, &vars).unwrap();
    assert!(path.exists());
}

#[test]
fn load_merged_env_global_only() {
    let dir = TempDir::new().unwrap();
    let mut global = BTreeMap::new();
    global.insert("TOKEN".to_string(), "abc".to_string());
    write_env_file(&global_env_path(dir.path()), &global).unwrap();

    let merged = load_merged_env(dir.path(), "myproject");
    assert_eq!(merged, vec![("TOKEN".to_string(), "abc".to_string())]);
}

#[test]
fn load_merged_env_project_only() {
    let dir = TempDir::new().unwrap();
    let mut project = BTreeMap::new();
    project.insert("DB_URL".to_string(), "postgres://...".to_string());
    write_env_file(&project_env_path(dir.path(), "myproject"), &project).unwrap();

    let merged = load_merged_env(dir.path(), "myproject");
    assert_eq!(
        merged,
        vec![("DB_URL".to_string(), "postgres://...".to_string())]
    );
}

#[test]
fn load_merged_env_project_overrides_global() {
    let dir = TempDir::new().unwrap();

    let mut global = BTreeMap::new();
    global.insert("TOKEN".to_string(), "global-token".to_string());
    global.insert("SHARED".to_string(), "from-global".to_string());
    write_env_file(&global_env_path(dir.path()), &global).unwrap();

    let mut project = BTreeMap::new();
    project.insert("TOKEN".to_string(), "project-token".to_string());
    project.insert("LOCAL".to_string(), "project-only".to_string());
    write_env_file(&project_env_path(dir.path(), "proj"), &project).unwrap();

    let merged = load_merged_env(dir.path(), "proj");
    let map: BTreeMap<_, _> = merged.into_iter().collect();

    assert_eq!(map.get("TOKEN"), Some(&"project-token".to_string()));
    assert_eq!(map.get("SHARED"), Some(&"from-global".to_string()));
    assert_eq!(map.get("LOCAL"), Some(&"project-only".to_string()));
}

#[test]
fn load_merged_env_empty_namespace_skips_project() {
    let dir = TempDir::new().unwrap();

    let mut global = BTreeMap::new();
    global.insert("TOKEN".to_string(), "abc".to_string());
    write_env_file(&global_env_path(dir.path()), &global).unwrap();

    // Project file exists but should be skipped because namespace is empty
    let mut project = BTreeMap::new();
    project.insert("TOKEN".to_string(), "should-not-appear".to_string());
    write_env_file(&project_env_path(dir.path(), ""), &project).unwrap();

    let merged = load_merged_env(dir.path(), "");
    let map: BTreeMap<_, _> = merged.into_iter().collect();
    assert_eq!(map.get("TOKEN"), Some(&"abc".to_string()));
}

#[test]
fn load_merged_env_no_files() {
    let dir = TempDir::new().unwrap();
    let merged = load_merged_env(dir.path(), "noproject");
    assert!(merged.is_empty());
}

#[test]
fn global_env_path_is_correct() {
    let dir = std::path::Path::new("/tmp/state");
    assert_eq!(
        global_env_path(dir),
        std::path::PathBuf::from("/tmp/state/env")
    );
}

#[test]
fn project_env_path_is_correct() {
    let dir = std::path::Path::new("/tmp/state");
    assert_eq!(
        project_env_path(dir, "oddjobs"),
        std::path::PathBuf::from("/tmp/state/env.oddjobs")
    );
}
