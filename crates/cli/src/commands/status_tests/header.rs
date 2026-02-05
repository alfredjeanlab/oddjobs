use serial_test::serial;

use super::*;

#[test]
#[serial]
fn without_watch_interval() {
    set_no_color();
    let out = format_text(120, &[], None);
    assert_eq!(out, "oj daemon: running 2m\n");
}

#[test]
#[serial]
fn with_watch_interval() {
    set_no_color();
    let out = format_text(120, &[], Some("5s"));
    assert_eq!(out, "oj daemon: running 2m | every 5s\n");
}

#[test]
#[serial]
fn with_custom_watch_interval() {
    set_no_color();
    let out = format_text(3700, &[], Some("10s"));
    assert_eq!(out, "oj daemon: running 1h1m | every 10s\n");
}

#[test]
#[serial]
fn with_active_jobs_and_watch() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "test".to_string(),
        active_jobs: vec![make_job("abc12345", "build", "job", "compile", "running")],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };
    let out = format_text(60, &[ns], Some("2s"));
    let first_line = out.lines().next().unwrap();
    assert_eq!(
        first_line,
        "oj daemon: running 1m | every 2s | 1 active job"
    );
}

#[test]
#[serial]
fn without_watch_has_no_every() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "test".to_string(),
        active_jobs: vec![make_job("abc12345", "build", "job", "compile", "running")],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };
    let out = format_text(60, &[ns], None);
    let first_line = out.lines().next().unwrap();
    assert_eq!(first_line, "oj daemon: running 1m | 1 active job");
    assert!(!first_line.contains("every"));
}

#[test]
#[serial]
fn decisions_pending_singular() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "test".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 1,
    };
    let out = format_text(60, &[ns], None);
    let first_line = out.lines().next().unwrap();
    assert!(
        first_line.contains("| 1 decision pending"),
        "header should show singular decision pending: {first_line}"
    );
    assert!(
        !first_line.contains("decisions"),
        "singular should not have trailing 's': {first_line}"
    );
}

#[test]
#[serial]
fn decisions_pending_plural() {
    set_no_color();

    let ns1 = NamespaceStatus {
        namespace: "proj-a".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 2,
    };
    let ns2 = NamespaceStatus {
        namespace: "proj-b".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 1,
    };
    let out = format_text(60, &[ns1, ns2], None);
    let first_line = out.lines().next().unwrap();
    assert!(
        first_line.contains("| 3 decisions pending"),
        "header should show total decisions pending across namespaces: {first_line}"
    );
}

#[test]
#[serial]
fn decisions_hidden_when_zero() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "test".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };
    let out = format_text(60, &[ns], None);
    assert!(
        !out.contains("decision"),
        "header should not mention decisions when count is zero: {out}"
    );
}
