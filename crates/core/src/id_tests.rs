// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use std::collections::HashMap;

crate::define_id! {
    /// Test ID type for macro verification.
    pub struct TestId;
}

#[test]
fn define_id_hash_map_lookup() {
    let mut map = HashMap::new();
    map.insert(TestId::new("k"), 42);
    assert_eq!(map.get("k"), Some(&42));
}

#[test]
fn define_id_short_truncates() {
    let id = TestId::new("abcdefghijklmnop");
    assert_eq!(id.short(8), "abcdefgh");
}

#[test]
fn define_id_short_returns_full_when_shorter() {
    let id = TestId::new("abc");
    assert_eq!(id.short(8), "abc");
}

#[test]
fn define_id_short_returns_full_when_exact() {
    let id = TestId::new("abcdefgh");
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
    let id = TestId::new("abc-123");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, r#""abc-123""#);

    let decoded: TestId = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, id);
}

// --- Deref tests ---

#[test]
fn id_derefs_to_str() {
    let id = TestId::new("test-id");
    let s: &str = &id; // Deref coercion
    assert_eq!(s, "test-id");
}

#[test]
fn option_id_as_deref() {
    let id = Some(TestId::new("test-id"));
    assert_eq!(id.as_deref(), Some("test-id"));

    let empty: Option<TestId> = None;
    assert_eq!(empty.as_deref(), None);
}

// --- IdGen tests ---

#[test]
fn uuid_gen_creates_unique_ids() {
    let id_gen = UuidIdGen;
    let id1 = id_gen.next();
    let id2 = id_gen.next();
    assert_ne!(id1, id2);
    assert_eq!(id1.len(), 36); // UUID format
}

#[test]
fn sequential_gen_creates_predictable_ids() {
    let id_gen = SequentialIdGen::new("test");
    assert_eq!(id_gen.next(), "test-1");
    assert_eq!(id_gen.next(), "test-2");
    assert_eq!(id_gen.next(), "test-3");
}

#[test]
fn sequential_gen_is_cloneable_and_shared() {
    let id_gen1 = SequentialIdGen::new("shared");
    let id_gen2 = id_gen1.clone();
    assert_eq!(id_gen1.next(), "shared-1");
    assert_eq!(id_gen2.next(), "shared-2");
    assert_eq!(id_gen1.next(), "shared-3");
}
