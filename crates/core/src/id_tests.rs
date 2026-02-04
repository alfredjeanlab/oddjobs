// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use std::borrow::Borrow;
use std::collections::HashMap;

// --- define_id! macro tests ---

crate::define_id! {
    /// Test ID type for macro verification.
    pub struct TestId;
}

#[test]
fn define_id_new_and_as_str() {
    let id = TestId::new("abc");
    assert_eq!(id.as_str(), "abc");
}

#[test]
fn define_id_display() {
    let id = TestId::new("hello");
    assert_eq!(format!("{}", id), "hello");
    assert_eq!(id.to_string(), "hello");
}

#[test]
fn define_id_from_string() {
    let id: TestId = String::from("owned").into();
    assert_eq!(id.as_str(), "owned");
}

#[test]
fn define_id_from_str() {
    let id: TestId = "borrowed".into();
    assert_eq!(id.as_str(), "borrowed");
}

#[test]
fn define_id_partial_eq_str() {
    let id = TestId::new("test");
    assert_eq!(id, *"test");
    assert_eq!(id, "test");
}

#[test]
fn define_id_borrow_str() {
    let id = TestId::new("key");
    let borrowed: &str = id.borrow();
    assert_eq!(borrowed, "key");
}

#[test]
fn define_id_hash_map_lookup() {
    let mut map = HashMap::new();
    map.insert(TestId::new("k"), 42);
    assert_eq!(map.get("k"), Some(&42));
}

#[test]
fn define_id_clone_and_eq() {
    let id = TestId::new("x");
    let cloned = id.clone();
    assert_eq!(id, cloned);
}

#[test]
fn define_id_serde_roundtrip() {
    let id = TestId::new("serde-test");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"serde-test\"");
    let deserialized: TestId = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, id);
}

// --- short() tests ---

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
fn short_id_trait_on_str() {
    use crate::id::ShortId;
    let s = "abcdefghijklmnop";
    assert_eq!(s.short(8), "abcdefgh");
    assert_eq!(s.short(100), s);
    assert_eq!("abc".short(8), "abc");
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
