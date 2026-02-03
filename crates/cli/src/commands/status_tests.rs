use oj_daemon::NamespaceStatus;
use serial_test::serial;

use super::{format_duration, format_text};

#[test]
#[serial]
fn header_without_watch_interval() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let out = format_text(120, &[], None);
    assert_eq!(out, "oj daemon: running 2m\n");
}

#[test]
#[serial]
fn header_with_watch_interval() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let out = format_text(120, &[], Some("5s"));
    assert_eq!(out, "oj daemon: running 2m | every 5s\n");
}

#[test]
#[serial]
fn header_with_custom_watch_interval() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let out = format_text(3700, &[], Some("10s"));
    assert_eq!(out, "oj daemon: running 1h1m | every 10s\n");
}

#[test]
#[serial]
fn header_with_active_pipelines_and_watch() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "test".to_string(),
        active_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "abc12345".to_string(),
            name: "build".to_string(),
            kind: "pipeline".to_string(),
            step: "compile".to_string(),
            step_status: "Running".to_string(),
            elapsed_ms: 5000,
            waiting_reason: None,
        }],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };
    let out = format_text(60, &[ns], Some("2s"));
    let first_line = out.lines().next().unwrap();
    assert_eq!(
        first_line,
        "oj daemon: running 1m | every 2s | 1 active pipeline"
    );
}

#[test]
#[serial]
fn header_without_watch_has_no_every() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "test".to_string(),
        active_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "abc12345".to_string(),
            name: "build".to_string(),
            kind: "pipeline".to_string(),
            step: "compile".to_string(),
            step_status: "Running".to_string(),
            elapsed_ms: 5000,
            waiting_reason: None,
        }],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };
    let out = format_text(60, &[ns], None);
    let first_line = out.lines().next().unwrap();
    assert_eq!(first_line, "oj daemon: running 1m | 1 active pipeline");
    assert!(!first_line.contains("every"));
}

#[test]
fn format_duration_values() {
    assert_eq!(format_duration(0), "0s");
    assert_eq!(format_duration(59), "59s");
    assert_eq!(format_duration(60), "1m");
    assert_eq!(format_duration(3599), "59m");
    assert_eq!(format_duration(3600), "1h");
    assert_eq!(format_duration(3660), "1h1m");
    assert_eq!(format_duration(86400), "1d");
}
