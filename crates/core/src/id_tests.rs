// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use std::collections::HashMap;

crate::define_id! {
    /// Test ID type for macro verification.
    pub struct TestId("tst-");
}

#[test]
fn id_has_prefix() {
    let id = TestId::from_string("tst-abc123");
    assert_eq!(id.as_str(), "tst-abc123");
    assert_eq!(id.suffix(), "abc123");
}

#[test]
fn id_short_uses_suffix() {
    let id = TestId::from_string("tst-abcdefgh");
    assert_eq!(id.short(3), "abc");
}

#[test]
fn id_generates_with_correct_length() {
    let id = TestId::new();
    assert_eq!(id.as_str().len(), 23);
    assert!(id.as_str().starts_with("tst-"));
}

#[test]
fn id_fits_in_smolstr_inline() {
    // Max inline: 4-char prefix + 19-char nanoid = 23 bytes
    let id = TestId::new();
    assert_eq!(id.as_str().len(), 23);
    assert!(id.as_str().len() <= 23); // Exactly fits inline capacity
}

#[test]
fn id_clones_cheaply() {
    let id1 = TestId::new();
    let id2 = id1.clone(); // Clones cheaply (24 bytes)
    assert_eq!(id1, id2);
}

#[test]
fn define_id_hash_map_lookup() {
    let mut map = HashMap::new();
    map.insert(TestId::from_string("tst-k"), 42);
    assert_eq!(map.get("tst-k"), Some(&42));
}

#[test]
fn define_id_short_truncates() {
    let id = TestId::from_string("tst-abcdefghijklmnop");
    assert_eq!(id.short(8), "abcdefgh");
}

#[test]
fn define_id_short_returns_full_when_shorter() {
    let id = TestId::from_string("tst-abc");
    assert_eq!(id.short(8), "abc");
}

#[test]
fn define_id_short_returns_full_when_exact() {
    let id = TestId::from_string("tst-abcdefgh");
    assert_eq!(id.short(8), "abcdefgh");
}

#[test]
fn short_fn_on_str() {
    let s = "abcdefghijklmnop";
    assert_eq!(short(s, 8), "abcdefgh");
    assert_eq!(short(s, 100), s);
    assert_eq!(short("abc", 8), "abc");
}

// --- Serialization tests ---

#[test]
fn id_serializes_as_bare_string() {
    let id = TestId::from_string("tst-abc123");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, r#""tst-abc123""#);

    let decoded: TestId = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, id);
}

// --- Deref tests ---

#[test]
fn id_derefs_to_str() {
    let id = TestId::from_string("tst-test-id");
    let s: &str = &id; // Deref coercion
    assert_eq!(s, "tst-test-id");
}

#[test]
fn option_id_as_deref() {
    let id = Some(TestId::from_string("tst-test-id"));
    assert_eq!(id.as_deref(), Some("tst-test-id"));

    let empty: Option<TestId> = None;
    assert_eq!(empty.as_deref(), None);
}
