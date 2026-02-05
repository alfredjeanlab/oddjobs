use serial_test::serial;

use super::*;

// ── ANSI constants ──────────────────────────────────────────────────

#[yare::parameterized(
    cursor_home = { CURSOR_HOME, "\x1B[H", 3 },
    clear_to_end = { CLEAR_TO_END, "\x1B[J", 3 },
    clear_to_eol = { CLEAR_TO_EOL, "\x1B[K", 3 },
)]
fn ansi_constant(value: &str, expected: &str, expected_len: usize) {
    assert_eq!(value, expected);
    assert_eq!(value.len(), expected_len);
}

// ── TTY vs non-TTY framing ──────────────────────────────────────────

#[test]
fn tty_wraps_with_cursor_home_and_clear() {
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
    let inner = &frame[CURSOR_HOME.len()..frame.len() - CLEAR_TO_END.len()];
    let expected = content.replace('\n', &format!("{CLEAR_TO_EOL}\n"));
    assert_eq!(inner, expected);
}

#[test]
fn non_tty_no_escape_codes() {
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
fn content_identical_across_tty_modes() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "proj".to_string(),
        active_jobs: vec![make_job("aaaa1111", "build", "job", "compile", "running")],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };
    let text = format_text(60, &[ns], Some("5s"));

    let tty_frame = render_frame(&text, true);
    let non_tty_frame = render_frame(&text, false);

    let inner = &tty_frame[CURSOR_HOME.len()..tty_frame.len() - CLEAR_TO_END.len()];
    let stripped = inner.replace(CLEAR_TO_EOL, "");
    assert_eq!(stripped, non_tty_frame);
}

#[test]
#[serial]
fn consecutive_frames_tty_each_have_escape_codes() {
    set_no_color();

    let frame1_content = format_text(60, &[], Some("5s"));
    let frame2_content = format_text(120, &[], Some("5s"));

    let frame1 = render_frame(&frame1_content, true);
    let frame2 = render_frame(&frame2_content, true);

    let combined = format!("{frame1}{frame2}");

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
    set_no_color();

    let frame1_content = format_text(60, &[], Some("5s"));
    let frame2_content = format_text(120, &[], Some("5s"));

    let frame1 = render_frame(&frame1_content, false);
    let frame2 = render_frame(&frame2_content, false);

    let combined = format!("{frame1}{frame2}");

    assert!(
        !combined.contains('\x1B'),
        "non-TTY output must never contain escape sequences"
    );
    assert!(combined.contains("1m")); // 60s
    assert!(combined.contains("2m")); // 120s
}

// ── Per-line clearing ───────────────────────────────────────────────

#[test]
fn tty_clears_each_line() {
    let content = "line one\nline two\nline three\n";
    let frame = render_frame(content, true);

    let eol_count = frame.matches(CLEAR_TO_EOL).count();
    let newline_count = content.matches('\n').count();
    assert_eq!(
        eol_count, newline_count,
        "each newline should have a preceding clear-to-EOL sequence"
    );

    for line in content.lines() {
        let pattern = format!("{line}{CLEAR_TO_EOL}\n");
        assert!(
            frame.contains(&pattern),
            "TTY frame should contain '{line}\\x1B[K\\n'"
        );
    }
}

#[test]
fn non_tty_has_no_eol_clearing() {
    let content = "line one\nline two\n";
    let frame = render_frame(content, false);
    assert!(
        !frame.contains(CLEAR_TO_EOL),
        "non-TTY frame should not contain clear-to-EOL sequences"
    );
}

#[test]
fn shorter_frame_clears_previous_line_remnants() {
    let short_content = "oj daemon: running 10m\n\
                          ── wok ──────────\n\
                          \x20   eeee1111  ci/lint  running  1s\n";

    let short_frame = render_frame(short_content, true);

    for line in short_content.lines() {
        let pattern = format!("{line}{CLEAR_TO_EOL}\n");
        assert!(
            short_frame.contains(&pattern),
            "short TTY frame must clear-to-EOL after: {line}"
        );
    }

    assert!(short_frame.starts_with(CURSOR_HOME));
    assert!(short_frame.ends_with(CLEAR_TO_END));

    let stripped = short_frame
        .strip_prefix(CURSOR_HOME)
        .unwrap()
        .strip_suffix(CLEAR_TO_END)
        .unwrap()
        .replace(CLEAR_TO_EOL, "");
    assert_eq!(stripped, short_content);
}

// ── Color / escape interaction ──────────────────────────────────────

#[test]
#[serial]
fn format_text_never_contains_escape_sequences() {
    set_no_color();

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
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![make_job("abcd1234", "build", "job", "compile", "running")],
        escalated_jobs: vec![{
            let mut j = make_job("efgh5678", "deploy", "deploy", "approve", "waiting");
            j.waiting_reason = Some("needs manual approval".to_string());
            j
        }],
        orphaned_jobs: vec![],
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
            agent_id: "agent-01".to_string(),
            status: "running".to_string(),
        }],
        pending_decisions: 0,
    };

    let text = format_text(600, &[ns], Some("5s"));
    let frame = render_frame(&text, false);

    assert!(
        !frame.contains('\x1B'),
        "no ANSI escapes in non-TTY + NO_COLOR frame"
    );
    assert!(frame.contains("myproject"));
    assert!(frame.contains("job"));
    assert!(frame.contains("builder"));
    assert!(frame.contains("tasks"));
    assert!(frame.contains("agent-01"));
}

#[test]
#[serial]
fn tty_frame_preserves_color_codes_in_content() {
    std::env::remove_var("NO_COLOR");
    std::env::set_var("COLOR", "1");

    let text = format_text(120, &[], Some("5s"));
    let frame = render_frame(&text, true);

    assert!(frame.starts_with(CURSOR_HOME));
    assert!(frame.ends_with(CLEAR_TO_END));

    let inner = &frame[CURSOR_HOME.len()..frame.len() - CLEAR_TO_END.len()];
    let stripped = inner.replace(CLEAR_TO_EOL, "");
    assert!(
        stripped.contains("\x1b[38;5;"),
        "TTY frame should preserve color codes from content"
    );
}
