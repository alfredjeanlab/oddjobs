// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

#[test]
fn starts_empty() {
    let tracker = ActionTracker::default();
    assert!(tracker.attempts.is_empty());
}

#[test]
fn increment_attempt() {
    let mut tracker = ActionTracker::default();

    assert_eq!(tracker.increment_attempt("idle", 0), 1);
    assert_eq!(tracker.increment_attempt("idle", 0), 2);
    assert_eq!(tracker.increment_attempt("idle", 0), 3);
}

#[test]
fn get_action_attempt() {
    let mut tracker = ActionTracker::default();

    assert_eq!(tracker.get_action_attempt("unknown", 0), 0);

    tracker.increment_attempt("idle", 0);
    assert_eq!(tracker.get_action_attempt("idle", 0), 1);

    tracker.increment_attempt("idle", 0);
    assert_eq!(tracker.get_action_attempt("idle", 0), 2);
}

#[test]
fn different_triggers_tracked_separately() {
    let mut tracker = ActionTracker::default();

    assert_eq!(tracker.increment_attempt("idle", 0), 1);
    assert_eq!(tracker.increment_attempt("exit", 0), 1);
    assert_eq!(tracker.increment_attempt("idle", 0), 2);
    assert_eq!(tracker.increment_attempt("exit", 0), 2);

    assert_eq!(tracker.get_action_attempt("idle", 0), 2);
    assert_eq!(tracker.get_action_attempt("exit", 0), 2);
}

#[test]
fn different_chain_positions_tracked_separately() {
    let mut tracker = ActionTracker::default();

    assert_eq!(tracker.increment_attempt("idle", 0), 1);
    assert_eq!(tracker.increment_attempt("idle", 1), 1);
    assert_eq!(tracker.increment_attempt("idle", 0), 2);

    assert_eq!(tracker.get_action_attempt("idle", 0), 2);
    assert_eq!(tracker.get_action_attempt("idle", 1), 1);
}

#[test]
fn reset_clears_all() {
    let mut tracker = ActionTracker::default();

    tracker.increment_attempt("idle", 0);
    tracker.increment_attempt("idle", 0);
    tracker.increment_attempt("exit", 0);

    tracker.reset_attempts();

    assert_eq!(tracker.get_action_attempt("idle", 0), 0);
    assert_eq!(tracker.get_action_attempt("exit", 0), 0);
    assert!(tracker.attempts.is_empty());
}

#[test]
fn serde_round_trip() {
    let mut tracker = ActionTracker::default();
    tracker.increment_attempt("on_idle", 0);
    tracker.increment_attempt("on_idle", 0);
    tracker.increment_attempt("on_fail", 1);

    let json = serde_json::to_string(&tracker).expect("serialize");
    let restored: ActionTracker = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(restored.get_action_attempt("on_idle", 0), 2);
    assert_eq!(restored.get_action_attempt("on_fail", 1), 1);
    assert_eq!(restored.get_action_attempt("unknown", 0), 0);
}
