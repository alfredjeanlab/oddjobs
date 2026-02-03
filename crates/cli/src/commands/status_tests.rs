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

#[test]
#[serial]
fn active_pipeline_shows_kind_not_name() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "abcd1234-0000-0000-0000".to_string(),
            name: "abcd1234-0000-0000-0000".to_string(),
            kind: "build".to_string(),
            step: "check".to_string(),
            step_status: "Running".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: None,
        }],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("build"),
        "output should contain pipeline kind 'build':\n{output}"
    );
    assert!(
        output.contains("check"),
        "output should contain step name 'check':\n{output}"
    );
    // The pipeline UUID name should not appear in the rendered text
    assert!(
        !output.contains("abcd1234-0000"),
        "output should not contain the UUID name:\n{output}"
    );
}

#[test]
#[serial]
fn escalated_pipeline_shows_kind_not_name() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "efgh5678".to_string(),
            name: "efgh5678-uuid".to_string(),
            kind: "deploy".to_string(),
            step: "test".to_string(),
            step_status: "Waiting".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: Some("gate check failed".to_string()),
        }],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("deploy"),
        "output should contain pipeline kind 'deploy':\n{output}"
    );
    assert!(
        output.contains("test"),
        "output should contain step name 'test':\n{output}"
    );
    assert!(
        output.contains("gate check failed"),
        "output should contain waiting reason:\n{output}"
    );
    assert!(
        !output.contains("efgh5678-uuid"),
        "output should not contain the UUID name:\n{output}"
    );
}

#[test]
#[serial]
fn orphaned_pipeline_shows_kind_not_name() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "ijkl9012".to_string(),
            name: "ijkl9012-uuid".to_string(),
            kind: "ci".to_string(),
            step: "lint".to_string(),
            step_status: "Running".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: None,
        }],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("ci"),
        "output should contain pipeline kind 'ci':\n{output}"
    );
    assert!(
        output.contains("lint"),
        "output should contain step name 'lint':\n{output}"
    );
    assert!(
        !output.contains("ijkl9012-uuid"),
        "output should not contain the UUID name:\n{output}"
    );
}
