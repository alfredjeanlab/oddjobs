// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn bare_keys_get_var_prefix() {
    let input: HashMap<String, String> = [("branch".into(), "main".into())].into_iter().collect();
    let result = namespace_vars(&input);
    assert_eq!(result.get("var.branch"), Some(&"main".to_string()));
    assert!(!result.contains_key("branch"));
}

#[test]
fn dotted_bare_keys_get_var_prefix() {
    let input: HashMap<String, String> = [("mr.branch".into(), "feat/x".into())]
        .into_iter()
        .collect();
    let result = namespace_vars(&input);
    assert_eq!(result.get("var.mr.branch"), Some(&"feat/x".to_string()));
}

#[test]
fn already_prefixed_keys_are_kept() {
    let input: HashMap<String, String> = [
        ("var.mr.branch".into(), "feat/x".into()),
        ("invoke.dir".into(), "/tmp".into()),
        ("workspace.root".into(), "/ws".into()),
        ("local.repo".into(), "/repo".into()),
        ("args.name".into(), "test".into()),
        ("item.id".into(), "abc".into()),
    ]
    .into_iter()
    .collect();
    let result = namespace_vars(&input);
    assert_eq!(result.get("var.mr.branch"), Some(&"feat/x".to_string()));
    assert_eq!(result.get("invoke.dir"), Some(&"/tmp".to_string()));
    assert!(!result.contains_key("var.var.mr.branch"));
    assert!(!result.contains_key("var.invoke.dir"));
}

#[test]
fn mixed_bare_and_prefixed() {
    let input: HashMap<String, String> = [
        ("title".into(), "hello".into()),
        ("var.mr.branch".into(), "feat/x".into()),
        ("workspace.nonce".into(), "abc123".into()),
    ]
    .into_iter()
    .collect();
    let result = namespace_vars(&input);
    assert_eq!(result.get("var.title"), Some(&"hello".to_string()));
    assert_eq!(result.get("var.mr.branch"), Some(&"feat/x".to_string()));
    assert_eq!(result.get("workspace.nonce"), Some(&"abc123".to_string()));
    assert!(!result.contains_key("var.var.mr.branch"));
}
