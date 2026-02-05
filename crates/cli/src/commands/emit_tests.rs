// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use oj_core::AgentSignalKind;

#[test]
fn parse_signal_payload_plain_complete() {
    let result = parse_signal_payload("complete").unwrap();
    assert_eq!(result.kind, AgentSignalKind::Complete);
    assert_eq!(result.message, None);
}

#[test]
fn parse_signal_payload_plain_escalate() {
    let result = parse_signal_payload("escalate").unwrap();
    assert_eq!(result.kind, AgentSignalKind::Escalate);
    assert_eq!(result.message, None);
}

#[test]
fn parse_signal_payload_plain_with_whitespace() {
    let result = parse_signal_payload("  complete  ").unwrap();
    assert_eq!(result.kind, AgentSignalKind::Complete);
}

#[test]
fn parse_signal_payload_json_kind_complete() {
    let result = parse_signal_payload(r#"{"kind": "complete"}"#).unwrap();
    assert_eq!(result.kind, AgentSignalKind::Complete);
    assert_eq!(result.message, None);
}

#[test]
fn parse_signal_payload_json_kind_escalate_with_message() {
    let result = parse_signal_payload(r#"{"kind": "escalate", "message": "need help"}"#).unwrap();
    assert_eq!(result.kind, AgentSignalKind::Escalate);
    assert_eq!(result.message, Some("need help".to_string()));
}

#[test]
fn parse_signal_payload_invalid_json_errors() {
    let result = parse_signal_payload("{action: complete}");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("invalid signal payload"), "got: {}", err);
}

#[test]
fn parse_signal_payload_plain_continue() {
    let result = parse_signal_payload("continue").unwrap();
    assert_eq!(result.kind, AgentSignalKind::Continue);
    assert_eq!(result.message, None);
}

#[test]
fn parse_signal_payload_unknown_string_errors() {
    let result = parse_signal_payload("foobar");
    assert!(result.is_err());
}
