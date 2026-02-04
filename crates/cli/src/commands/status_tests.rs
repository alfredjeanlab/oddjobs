use oj_daemon::NamespaceStatus;
use serial_test::serial;

use super::{
    format_duration, format_text, friendly_name_label, render_frame, truncate_reason, CLEAR_TO_END,
    CURSOR_HOME,
};

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
            step_status: "running".to_string(),
            elapsed_ms: 5000,
            waiting_reason: None,
            escalate_source: None,
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
            step_status: "running".to_string(),
            elapsed_ms: 5000,
            waiting_reason: None,
            escalate_source: None,
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
            step_status: "running".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: None,
            escalate_source: None,
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
fn active_pipeline_hides_nonce_only_name() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    // When a name template produces an empty slug, pipeline_display_name()
    // returns just the nonce (first 8 chars of the ID). This should be hidden
    // since it's redundant with the truncated ID already shown in the output.
    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "abcd1234-0000-0000-0000".to_string(),
            name: "abcd1234".to_string(), // nonce-only name (first 8 chars of ID)
            kind: "build".to_string(),
            step: "check".to_string(),
            step_status: "running".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: None,
            escalate_source: None,
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
    // The nonce should only appear once (as the truncated ID), not twice
    // Count occurrences: split by the nonce and count segments
    let nonce_count = output.matches("abcd1234").count();
    assert_eq!(
        nonce_count, 1,
        "nonce 'abcd1234' should appear exactly once (as truncated ID), not twice:\n{output}"
    );
}

#[test]
#[serial]
fn escalated_pipeline_hides_name_when_same_as_id() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "efgh5678-0000-0000-0000".to_string(),
            name: "efgh5678-0000-0000-0000".to_string(),
            kind: "deploy".to_string(),
            step: "test".to_string(),
            step_status: "waiting".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: Some("gate check failed".to_string()),
            escalate_source: None,
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
    // The pipeline UUID name should not appear in the rendered text
    assert!(
        !output.contains("efgh5678-0000"),
        "output should not contain the UUID name:\n{output}"
    );
}

#[test]
#[serial]
fn orphaned_pipeline_hides_name_when_same_as_id() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "ijkl9012-0000-0000-0000".to_string(),
            name: "ijkl9012-0000-0000-0000".to_string(),
            kind: "ci".to_string(),
            step: "lint".to_string(),
            step_status: "running".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: None,
            escalate_source: None,
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
    // The pipeline UUID name should not appear in the rendered text
    assert!(
        !output.contains("ijkl9012-0000"),
        "output should not contain the UUID name:\n{output}"
    );
}

#[test]
fn truncate_reason_short_unchanged() {
    assert_eq!(
        truncate_reason("gate check failed", 72),
        "gate check failed"
    );
}

#[test]
fn truncate_reason_long_single_line() {
    let long = "a".repeat(100);
    let result = truncate_reason(&long, 72);
    assert_eq!(result.len(), 72);
    assert!(result.ends_with("..."));
    assert_eq!(result, format!("{}...", "a".repeat(69)));
}

#[test]
fn truncate_reason_multiline_takes_first_line() {
    let reason = "first line\nsecond line\nthird line";
    let result = truncate_reason(reason, 72);
    assert_eq!(result, "first line...");
    assert!(!result.contains("second"));
}

#[test]
fn truncate_reason_multiline_long_first_line() {
    let first_line = "x".repeat(100);
    let reason = format!("{}\nsecond line", first_line);
    let result = truncate_reason(&reason, 72);
    assert_eq!(result.len(), 72);
    assert!(result.ends_with("..."));
    assert_eq!(result, format!("{}...", "x".repeat(69)));
}

#[test]
#[serial]
fn escalated_pipeline_truncates_long_reason() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let long_reason = "e".repeat(200);
    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "efgh5678".to_string(),
            name: "efgh5678".to_string(),
            kind: "deploy".to_string(),
            step: "test".to_string(),
            step_status: "Waiting".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: Some(long_reason.clone()),
            escalate_source: None,
        }],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    // The full 200-char reason should NOT appear
    assert!(
        !output.contains(&long_reason),
        "output should not contain the full long reason:\n{output}"
    );
    // Should contain the truncated version with "..."
    assert!(
        output.contains("..."),
        "output should contain truncation indicator '...':\n{output}"
    );
}

#[test]
fn friendly_name_label_empty_when_name_equals_kind() {
    assert_eq!(friendly_name_label("build", "build", "abc123"), "");
}

#[test]
fn friendly_name_label_empty_when_name_equals_id() {
    assert_eq!(friendly_name_label("abc123", "build", "abc123"), "");
}

#[test]
fn friendly_name_label_empty_when_name_equals_truncated_id() {
    // When the name template produces an empty slug, pipeline_display_name()
    // returns just the nonce (first 8 chars of the ID). This should be hidden
    // since it's redundant with the truncated ID shown in status output.
    assert_eq!(
        friendly_name_label("abcd1234", "build", "abcd1234-0000-0000-0000"),
        ""
    );
}

#[test]
fn friendly_name_label_empty_when_name_is_empty() {
    assert_eq!(friendly_name_label("", "build", "abc123"), "");
}

#[test]
fn friendly_name_label_shown_when_meaningful() {
    assert_eq!(
        friendly_name_label("fix-login-button-a1b2c3d4", "build", "abc123"),
        "fix-login-button-a1b2c3d4"
    );
}

#[test]
#[serial]
fn active_pipeline_shows_friendly_name() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "abcd1234-0000-0000-0000".to_string(),
            name: "fix-login-button-abcd1234".to_string(),
            kind: "build".to_string(),
            step: "check".to_string(),
            step_status: "running".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: None,
            escalate_source: None,
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
        output.contains("fix-login-button-abcd1234"),
        "output should contain friendly name:\n{output}"
    );
    assert!(
        output.contains("check"),
        "output should contain step name 'check':\n{output}"
    );
}

#[test]
#[serial]
fn escalated_pipeline_shows_friendly_name() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "efgh5678-0000-0000-0000".to_string(),
            name: "deploy-staging-efgh5678".to_string(),
            kind: "deploy".to_string(),
            step: "test".to_string(),
            step_status: "waiting".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: Some("gate check failed".to_string()),
            escalate_source: None,
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
        output.contains("deploy-staging-efgh5678"),
        "output should contain friendly name:\n{output}"
    );
}

#[test]
#[serial]
fn orphaned_pipeline_shows_friendly_name() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "ijkl9012-0000-0000-0000".to_string(),
            name: "ci-main-branch-ijkl9012".to_string(),
            kind: "ci".to_string(),
            step: "lint".to_string(),
            step_status: "running".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: None,
            escalate_source: None,
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
        output.contains("ci-main-branch-ijkl9012"),
        "output should contain friendly name:\n{output}"
    );
}

// ── render_frame tests ──────────────────────────────────────────────

#[test]
fn render_frame_tty_wraps_with_cursor_home_and_clear() {
    let content = "oj daemon: running 2m\n";
    let frame = render_frame(content, true);
    assert!(
        frame.starts_with(CURSOR_HOME),
        "TTY frame must start with cursor-home sequence"
    );
    assert!(
        frame.ends_with(CLEAR_TO_END),
        "TTY frame must end with clear-to-end sequence"
    );
    // Content is sandwiched between the escape codes
    let inner = &frame[CURSOR_HOME.len()..frame.len() - CLEAR_TO_END.len()];
    assert_eq!(inner, content);
}

#[test]
fn render_frame_non_tty_no_escape_codes() {
    let content = "oj daemon: running 2m\n";
    let frame = render_frame(content, false);
    assert_eq!(frame, content, "non-TTY frame should be the raw content");
    assert!(
        !frame.contains('\x1B'),
        "non-TTY frame must not contain any ANSI escape codes"
    );
}

#[test]
#[serial]
fn render_frame_content_identical_across_tty_modes() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "proj".to_string(),
        active_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "aaaa1111".to_string(),
            name: "build".to_string(),
            kind: "pipeline".to_string(),
            step: "compile".to_string(),
            step_status: "running".to_string(),
            elapsed_ms: 5000,
            waiting_reason: None,
            escalate_source: None,
        }],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };
    let text = format_text(60, &[ns], Some("5s"));

    let tty_frame = render_frame(&text, true);
    let non_tty_frame = render_frame(&text, false);

    // Strip the wrapping escape codes from TTY frame; remainder must match non-TTY
    let inner = &tty_frame[CURSOR_HOME.len()..tty_frame.len() - CLEAR_TO_END.len()];
    assert_eq!(inner, non_tty_frame);
}

#[test]
#[serial]
fn consecutive_frames_tty_each_have_escape_codes() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let frame1_content = format_text(60, &[], Some("5s"));
    let frame2_content = format_text(120, &[], Some("5s"));

    let frame1 = render_frame(&frame1_content, true);
    let frame2 = render_frame(&frame2_content, true);

    let combined = format!("{frame1}{frame2}");

    // Count occurrences of the cursor-home sequence
    let home_count = combined.matches(CURSOR_HOME).count();
    assert_eq!(
        home_count, 2,
        "each TTY frame must have its own cursor-home"
    );

    let clear_count = combined.matches(CLEAR_TO_END).count();
    assert_eq!(
        clear_count, 2,
        "each TTY frame must have its own clear-to-end"
    );
}

#[test]
#[serial]
fn consecutive_frames_non_tty_no_escape_codes() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let frame1_content = format_text(60, &[], Some("5s"));
    let frame2_content = format_text(120, &[], Some("5s"));

    let frame1 = render_frame(&frame1_content, false);
    let frame2 = render_frame(&frame2_content, false);

    let combined = format!("{frame1}{frame2}");

    assert!(
        !combined.contains('\x1B'),
        "non-TTY output must never contain escape sequences"
    );
    // Both frames appear in order
    assert!(combined.contains("1m")); // 60s
    assert!(combined.contains("2m")); // 120s
}

#[test]
fn cursor_home_constant_is_correct_ansi() {
    assert_eq!(CURSOR_HOME, "\x1B[H");
    assert_eq!(CURSOR_HOME.len(), 3);
}

#[test]
fn clear_to_end_constant_is_correct_ansi() {
    assert_eq!(CLEAR_TO_END, "\x1B[J");
    assert_eq!(CLEAR_TO_END.len(), 3);
}

#[test]
#[serial]
fn format_text_never_contains_escape_sequences() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let with_watch = format_text(300, &[], Some("3s"));
    assert!(
        !with_watch.contains('\x1B'),
        "format_text must not inject escape codes"
    );

    let without_watch = format_text(300, &[], None);
    assert!(
        !without_watch.contains('\x1B'),
        "format_text must not inject escape codes"
    );
}

#[test]
#[serial]
fn non_tty_frame_with_full_status_has_no_ansi_escapes() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "abcd1234".to_string(),
            name: "build".to_string(),
            kind: "pipeline".to_string(),
            step: "compile".to_string(),
            step_status: "running".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: None,
            escalate_source: None,
        }],
        escalated_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "efgh5678".to_string(),
            name: "deploy".to_string(),
            kind: "deploy".to_string(),
            step: "approve".to_string(),
            step_status: "waiting".to_string(),
            elapsed_ms: 120_000,
            waiting_reason: Some("needs manual approval".to_string()),
            escalate_source: None,
        }],
        orphaned_pipelines: vec![],
        workers: vec![oj_daemon::WorkerSummary {
            name: "builder".to_string(),
            namespace: "myproject".to_string(),
            queue: "default".to_string(),
            status: "running".to_string(),
            active: 1,
            concurrency: 4,
            updated_at_ms: 0,
        }],
        queues: vec![oj_daemon::QueueStatus {
            name: "tasks".to_string(),
            pending: 3,
            active: 1,
            dead: 0,
        }],
        active_agents: vec![oj_daemon::AgentStatusEntry {
            agent_name: "coder".to_string(),
            command_name: "build".to_string(),
            agent_id: "agent-001".to_string(),
            status: "running".to_string(),
        }],
    };

    let text = format_text(600, &[ns], Some("5s"));
    let frame = render_frame(&text, false);

    assert!(
        !frame.contains('\x1B'),
        "no ANSI escapes in non-TTY + NO_COLOR frame"
    );
    assert!(frame.contains("myproject"));
    assert!(frame.contains("pipeline"));
    assert!(frame.contains("builder"));
    assert!(frame.contains("tasks"));
    assert!(frame.contains("agent-001"));
}

#[test]
#[serial]
fn tty_frame_preserves_color_codes_in_content() {
    std::env::remove_var("NO_COLOR");
    std::env::set_var("COLOR", "1");

    let text = format_text(120, &[], Some("5s"));
    let frame = render_frame(&text, true);

    // Starts with cursor-home sequence
    assert!(frame.starts_with(CURSOR_HOME));

    // Ends with clear-to-end sequence
    assert!(frame.ends_with(CLEAR_TO_END));

    // Contains color codes from format_text (header coloring)
    let inner = &frame[CURSOR_HOME.len()..frame.len() - CLEAR_TO_END.len()];
    assert!(
        inner.contains("\x1b[38;5;"),
        "TTY frame should preserve color codes from content"
    );
}

#[test]
#[serial]
fn namespace_with_only_empty_queues_is_hidden() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    // A namespace with only queues that have all-zero counts should not be displayed
    let ns = NamespaceStatus {
        namespace: "empty-project".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![oj_daemon::QueueStatus {
            name: "tasks".to_string(),
            pending: 0,
            active: 0,
            dead: 0,
        }],
        active_agents: vec![],
    };

    let output = format_text(60, &[ns], None);

    // The namespace header should not appear since there's no displayable content
    assert!(
        !output.contains("empty-project"),
        "namespace with only empty queues should be hidden:\n{output}"
    );
    // Only the daemon header should appear
    assert_eq!(output, "oj daemon: running 1m\n");
}

#[test]
#[serial]
fn namespace_with_non_empty_queue_is_shown() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    // A namespace with at least one queue with non-zero counts should be displayed
    let ns = NamespaceStatus {
        namespace: "active-project".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![oj_daemon::QueueStatus {
            name: "tasks".to_string(),
            pending: 1,
            active: 0,
            dead: 0,
        }],
        active_agents: vec![],
    };

    let output = format_text(60, &[ns], None);

    assert!(
        output.contains("active-project"),
        "namespace with non-empty queue should be shown:\n{output}"
    );
    assert!(
        output.contains("tasks"),
        "queue should be displayed:\n{output}"
    );
}

#[test]
#[serial]
fn escalated_pipeline_shows_source_label() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "efgh5678-0000-0000-0000".to_string(),
            name: "deploy-staging-efgh5678".to_string(),
            kind: "deploy".to_string(),
            step: "test".to_string(),
            step_status: "waiting".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: Some("Agent is idle".to_string()),
            escalate_source: Some("idle".to_string()),
        }],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("[idle]"),
        "output should contain source label '[idle]':\n{output}"
    );
}

#[test]
#[serial]
fn escalated_pipeline_no_source_label_when_none() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "efgh5678-0000-0000-0000".to_string(),
            name: "deploy-staging-efgh5678".to_string(),
            kind: "deploy".to_string(),
            step: "test".to_string(),
            step_status: "waiting".to_string(),
            elapsed_ms: 60_000,
            waiting_reason: Some("gate check failed".to_string()),
            escalate_source: None,
        }],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    assert!(
        !output.contains('['),
        "output should not contain bracket source label when source is None:\n{output}"
    );
}

#[test]
#[serial]
fn column_order_is_id_name_kindstep_status_elapsed() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![oj_daemon::PipelineStatusEntry {
            id: "abcd1234-0000-0000-0000".to_string(),
            name: "fix-login-abcd1234".to_string(),
            kind: "build".to_string(),
            step: "check".to_string(),
            step_status: "running".to_string(),
            elapsed_ms: 420_000,
            waiting_reason: None,
            escalate_source: None,
        }],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    // Find the pipeline row line
    let line = output
        .lines()
        .find(|l| l.contains("abcd1234"))
        .expect("should find pipeline row");

    // Verify column order: id, then name, then kind/step, then status, then elapsed
    let id_pos = line.find("abcd1234").unwrap();
    let name_pos = line.find("fix-login-abcd1234").unwrap();
    let kind_step_pos = line.find("build/check").unwrap();
    let status_pos = line.find("running").unwrap();
    let elapsed_pos = line.find("7m").unwrap();

    assert!(id_pos < name_pos, "id should come before name: {line}");
    assert!(
        name_pos < kind_step_pos,
        "name should come before kind/step: {line}"
    );
    assert!(
        kind_step_pos < status_pos,
        "kind/step should come before status: {line}"
    );
    assert!(
        status_pos < elapsed_pos,
        "status should come before elapsed: {line}"
    );
}

#[test]
#[serial]
fn columns_are_aligned_across_rows() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![
            oj_daemon::PipelineStatusEntry {
                id: "aaaa1111-0000".to_string(),
                name: "short-aaaa1111".to_string(),
                kind: "build".to_string(),
                step: "check".to_string(),
                step_status: "running".to_string(),
                elapsed_ms: 60_000,
                waiting_reason: None,
                escalate_source: None,
            },
            oj_daemon::PipelineStatusEntry {
                id: "bbbb2222-0000".to_string(),
                name: "much-longer-name-bbbb2222".to_string(),
                kind: "deploy".to_string(),
                step: "implement".to_string(),
                step_status: "waiting".to_string(),
                elapsed_ms: 120_000,
                waiting_reason: None,
                escalate_source: None,
            },
        ],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    let lines: Vec<&str> = output
        .lines()
        .filter(|l| l.contains("aaaa1111") || l.contains("bbbb2222"))
        .collect();
    assert_eq!(lines.len(), 2, "should find exactly 2 pipeline rows");

    // The kind/step column should start at the same position in both lines
    let ks_pos_0 = lines[0].find("build/check").unwrap();
    let ks_pos_1 = lines[1].find("deploy/implement").unwrap();
    assert_eq!(
        ks_pos_0, ks_pos_1,
        "kind/step columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );

    // The status column should also be aligned
    let st_pos_0 = lines[0].find("running").unwrap();
    let st_pos_1 = lines[1].find("waiting").unwrap();
    assert_eq!(
        st_pos_0, st_pos_1,
        "status columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );
}

#[test]
#[serial]
fn name_column_omitted_when_all_names_hidden() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![
            oj_daemon::PipelineStatusEntry {
                id: "aaaa1111-0000-0000-0000".to_string(),
                name: "aaaa1111-0000-0000-0000".to_string(), // same as id → hidden
                kind: "build".to_string(),
                step: "check".to_string(),
                step_status: "running".to_string(),
                elapsed_ms: 60_000,
                waiting_reason: None,
                escalate_source: None,
            },
            oj_daemon::PipelineStatusEntry {
                id: "bbbb2222-0000-0000-0000".to_string(),
                name: "build".to_string(), // same as kind → hidden
                kind: "build".to_string(),
                step: "test".to_string(),
                step_status: "running".to_string(),
                elapsed_ms: 120_000,
                waiting_reason: None,
                escalate_source: None,
            },
        ],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    let line = output
        .lines()
        .find(|l| l.contains("aaaa1111"))
        .expect("should find first pipeline row");

    // With all names hidden, the kind/step should follow closely after the id
    // (just 2 spaces separator, no extra name column padding)
    let id_end = line.find("aaaa1111").unwrap() + "aaaa1111".len();
    let ks_start = line.find("build/check").unwrap();
    assert_eq!(
        ks_start - id_end,
        2,
        "kind/step should follow id with just 2-space separator when names are hidden:\n  {line}"
    );
}

#[test]
#[serial]
fn worker_columns_are_aligned_across_rows() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![
            oj_daemon::WorkerSummary {
                name: "a".to_string(),
                namespace: "myproject".to_string(),
                queue: "default".to_string(),
                status: "running".to_string(),
                active: 1,
                concurrency: 4,
                updated_at_ms: 0,
            },
            oj_daemon::WorkerSummary {
                name: "long-worker-name".to_string(),
                namespace: "myproject".to_string(),
                queue: "default".to_string(),
                status: "idle".to_string(),
                active: 0,
                concurrency: 2,
                updated_at_ms: 0,
            },
        ],
        queues: vec![],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    let lines: Vec<&str> = output.lines().filter(|l| l.contains("active")).collect();
    assert_eq!(lines.len(), 2, "should find exactly 2 worker rows");

    // The indicator (● or ○) should start at the same position in both lines
    let ind_pos_0 = lines[0].find('●').or_else(|| lines[0].find('○')).unwrap();
    let ind_pos_1 = lines[1].find('●').or_else(|| lines[1].find('○')).unwrap();
    assert_eq!(
        ind_pos_0, ind_pos_1,
        "indicator columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );
}

#[test]
#[serial]
fn queue_columns_are_aligned_across_rows() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![
            oj_daemon::QueueStatus {
                name: "tasks".to_string(),
                pending: 3,
                active: 1,
                dead: 0,
            },
            oj_daemon::QueueStatus {
                name: "long-queue-name".to_string(),
                pending: 12,
                active: 2,
                dead: 1,
            },
        ],
        active_agents: vec![],
    };

    let output = format_text(30, &[ns], None);

    let lines: Vec<&str> = output.lines().filter(|l| l.contains("pending")).collect();
    assert_eq!(lines.len(), 2, "should find exactly 2 queue rows");

    // The "pending" counts should start at the same column position
    let pending_pos_0 = lines[0].find("pending").unwrap();
    let pending_pos_1 = lines[1].find("pending").unwrap();

    // The number before "pending" may differ in length, but the name column
    // should be padded so the number starts at the same position
    let num_start_0 = lines[0].find(|c: char| c.is_ascii_digit()).unwrap();
    let num_start_1 = lines[1].find(|c: char| c.is_ascii_digit()).unwrap();
    assert_eq!(
        num_start_0, num_start_1,
        "count columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );

    // Sanity check: both "pending" positions differ because the numbers differ
    // but the start of the number column should be the same
    let _ = pending_pos_0;
    let _ = pending_pos_1;
}

#[test]
#[serial]
fn agent_columns_are_aligned_across_rows() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_pipelines: vec![],
        escalated_pipelines: vec![],
        orphaned_pipelines: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![
            oj_daemon::AgentStatusEntry {
                agent_name: "coder".to_string(),
                command_name: "build".to_string(),
                agent_id: "agent-001".to_string(),
                status: "running".to_string(),
            },
            oj_daemon::AgentStatusEntry {
                agent_name: "long-agent-name".to_string(),
                command_name: "deploy".to_string(),
                agent_id: "agent-002".to_string(),
                status: "idle".to_string(),
            },
        ],
    };

    let output = format_text(30, &[ns], None);

    let lines: Vec<&str> = output.lines().filter(|l| l.contains("agent-")).collect();
    assert_eq!(lines.len(), 2, "should find exactly 2 agent rows");

    // The agent_id column should start at the same position in both lines
    let id_pos_0 = lines[0].find("agent-001").unwrap();
    let id_pos_1 = lines[1].find("agent-002").unwrap();
    assert_eq!(
        id_pos_0, id_pos_1,
        "agent_id columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );

    // The status column should also be aligned
    let st_pos_0 = lines[0].find("running").unwrap();
    let st_pos_1 = lines[1].find("idle").unwrap();
    assert_eq!(
        st_pos_0, st_pos_1,
        "status columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );
}
