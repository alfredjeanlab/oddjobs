// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use oj_runbook::QueueType;

// ============================================================================
// Queue Type
// ============================================================================

#[test]
fn external_queue_with_explicit_type() {
    let hcl = r#"
queue "bugs" {
  type = "external"
  list = "wok list -t bug -o json"
  take = "wok start ${item.id}"
}
"#;
    let queue = &super::parse_hcl(hcl).queues["bugs"];
    assert_eq!(queue.queue_type, QueueType::External);
    assert_eq!(queue.list.as_deref(), Some("wok list -t bug -o json"));
    assert_eq!(queue.take.as_deref(), Some("wok start ${item.id}"));
}

#[test]
fn persisted_queue() {
    let hcl = r#"
queue "merges" {
  type     = "persisted"
  vars     = ["branch", "title", "base"]
  defaults = { base = "main" }
}
"#;
    let queue = &super::parse_hcl(hcl).queues["merges"];
    assert_eq!(queue.queue_type, QueueType::Persisted);
    assert_eq!(queue.vars, vec!["branch", "title", "base"]);
    assert_eq!(queue.defaults.get("base"), Some(&"main".to_string()));
    assert!(queue.list.is_none());
    assert!(queue.take.is_none());
}

#[test]
fn queue_defaults_to_external() {
    let hcl = "queue \"items\" {\n  list = \"echo '[]'\"\n  take = \"echo ok\"\n}";
    assert_eq!(
        super::parse_hcl(hcl).queues["items"].queue_type,
        QueueType::External
    );
}

#[test]
fn error_external_missing_list() {
    super::assert_hcl_err(
        "queue \"items\" {\n  type = \"external\"\n  take = \"echo ok\"\n}",
        &["external queue requires 'list' field"],
    );
}

#[test]
fn error_external_missing_take() {
    super::assert_hcl_err(
        "queue \"items\" {\n  type = \"external\"\n  list = \"echo '[]'\"\n}",
        &["external queue requires 'take' field"],
    );
}

#[test]
fn error_persisted_missing_vars() {
    super::assert_hcl_err(
        "queue \"items\" {\n  type = \"persisted\"\n}",
        &["persisted queue requires 'vars' field"],
    );
}

#[test]
fn error_persisted_with_list() {
    super::assert_hcl_err(
        "queue \"items\" {\n  type = \"persisted\"\n  vars = [\"branch\"]\n  list = \"echo '[]'\"\n}",
        &["persisted queue must not have 'list' field"],
    );
}

#[test]
fn error_persisted_with_take() {
    super::assert_hcl_err(
        "queue \"items\" {\n  type = \"persisted\"\n  vars = [\"branch\"]\n  take = \"echo ok\"\n}",
        &["persisted queue must not have 'take' field"],
    );
}

// ============================================================================
// Queue Poll
// ============================================================================

#[test]
fn external_queue_with_poll() {
    let hcl = r#"
queue "bugs" {
  type = "external"
  list = "wok list -t bug -o json"
  take = "wok start ${item.id}"
  poll = "30s"
}
"#;
    let queue = &super::parse_hcl(hcl).queues["bugs"];
    assert_eq!(queue.queue_type, QueueType::External);
    assert_eq!(queue.poll.as_deref(), Some("30s"));
}

#[test]
fn external_queue_with_poll_millis() {
    let hcl = r#"
queue "fast" {
  type = "external"
  list = "echo '[]'"
  take = "echo ok"
  poll = "200ms"
}
"#;
    assert_eq!(
        super::parse_hcl(hcl).queues["fast"].poll.as_deref(),
        Some("200ms")
    );
}

#[test]
fn external_queue_without_poll() {
    let hcl =
        "queue \"bugs\" {\n  type = \"external\"\n  list = \"echo '[]'\"\n  take = \"echo ok\"\n}";
    assert!(super::parse_hcl(hcl).queues["bugs"].poll.is_none());
}

#[test]
fn error_external_with_invalid_poll() {
    let hcl = r#"
queue "bugs" {
  type = "external"
  list = "echo '[]'"
  take = "echo ok"
  poll = "bogus"
}
"#;
    super::assert_hcl_err(hcl, &["queue.bugs.poll"]);
}

#[test]
fn error_persisted_with_poll() {
    super::assert_hcl_err(
        "queue \"items\" {\n  type = \"persisted\"\n  vars = [\"branch\"]\n  poll = \"30s\"\n}",
        &["persisted queue must not have 'poll' field"],
    );
}
