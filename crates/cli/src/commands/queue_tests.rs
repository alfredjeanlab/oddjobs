// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::{build_data_map, parse_key_value};
use serde_json::json;

#[test]
fn parse_key_value_simple() {
    let (k, v) = parse_key_value("branch=main").unwrap();
    assert_eq!(k, "branch");
    assert_eq!(v, "main");
}

#[test]
fn parse_key_value_with_equals_in_value() {
    let (k, v) = parse_key_value("expr=a=b").unwrap();
    assert_eq!(k, "expr");
    assert_eq!(v, "a=b");
}

#[test]
fn parse_key_value_empty_value() {
    let (k, v) = parse_key_value("key=").unwrap();
    assert_eq!(k, "key");
    assert_eq!(v, "");
}

#[test]
fn parse_key_value_missing_equals() {
    let err = parse_key_value("noequals").unwrap_err();
    assert!(err.contains("must be key=value"));
}

#[test]
fn build_data_map_vars_only() {
    let result = build_data_map(
        None,
        vec![
            ("branch".into(), "fix-123".into()),
            ("title".into(), "fix: something".into()),
        ],
    )
    .unwrap();

    assert_eq!(
        result,
        json!({"branch": "fix-123", "title": "fix: something"})
    );
}

#[test]
fn build_data_map_json_only() {
    let result = build_data_map(
        Some(r#"{"branch": "fix-123", "title": "fix: something"}"#.into()),
        vec![],
    )
    .unwrap();

    assert_eq!(
        result,
        json!({"branch": "fix-123", "title": "fix: something"})
    );
}

#[test]
fn build_data_map_merge_var_overrides_json() {
    let result = build_data_map(
        Some(r#"{"branch": "old", "base": "main"}"#.into()),
        vec![("branch".into(), "new".into())],
    )
    .unwrap();

    assert_eq!(result, json!({"branch": "new", "base": "main"}));
}

#[test]
fn build_data_map_merge_var_adds_to_json() {
    let result = build_data_map(
        Some(r#"{"branch": "fix-123"}"#.into()),
        vec![("title".into(), "fix: bug".into())],
    )
    .unwrap();

    assert_eq!(result, json!({"branch": "fix-123", "title": "fix: bug"}));
}

#[test]
fn build_data_map_empty_errors() {
    let err = build_data_map(None, vec![]).unwrap_err();
    assert!(err.to_string().contains("no data provided"));
}

#[test]
fn build_data_map_invalid_json() {
    let err = build_data_map(Some("not json".into()), vec![]).unwrap_err();
    assert!(err.to_string().contains("invalid JSON data"));
}

#[test]
fn build_data_map_json_not_object() {
    let err = build_data_map(Some("[1, 2, 3]".into()), vec![]).unwrap_err();
    assert!(err.to_string().contains("JSON data must be an object"));
}
