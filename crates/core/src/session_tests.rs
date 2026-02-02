// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn session_id_display() {
    let id = SessionId::new("test-session");
    assert_eq!(id.to_string(), "test-session");
}

#[test]
fn session_id_equality() {
    let id1 = SessionId::new("session-1");
    let id2 = SessionId::new("session-1");
    let id3 = SessionId::new("session-2");

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
}

#[test]
fn session_id_from_str() {
    let id: SessionId = "test".into();
    assert_eq!(id.as_str(), "test");
}

#[test]
fn session_id_serde() {
    let id = SessionId::new("my-session");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"my-session\"");

    let parsed: SessionId = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, id);
}
