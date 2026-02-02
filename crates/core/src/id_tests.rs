// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

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
