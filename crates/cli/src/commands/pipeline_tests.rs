// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::{
    format_pipeline_list, format_var_value, parse_duration, print_step_progress, status_group,
    StepTracker,
};
use oj_daemon::{PipelineDetail, PipelineSummary, StepRecordDetail};
use std::collections::HashMap;
use std::time::Duration;

#[test]
fn parse_duration_seconds() {
    assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
}

#[test]
fn parse_duration_minutes() {
    assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
}

#[test]
fn parse_duration_hours() {
    assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
}

#[test]
fn parse_duration_combined() {
    assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
}

#[test]
fn parse_duration_bare_number() {
    assert_eq!(parse_duration("60").unwrap(), Duration::from_secs(60));
}

#[test]
fn parse_duration_zero_fails() {
    assert!(parse_duration("0s").is_err());
}

#[test]
fn status_group_running_is_active() {
    assert_eq!(status_group("build", "Running"), 0);
}

#[test]
fn status_group_pending_is_active() {
    assert_eq!(status_group("execute", "Pending"), 0);
}

#[test]
fn status_group_waiting_is_active() {
    assert_eq!(status_group("review", "Waiting"), 0);
}

#[test]
fn status_group_failed_step() {
    assert_eq!(status_group("failed", "Failed"), 1);
}

#[test]
fn status_group_failed_step_any_status() {
    assert_eq!(status_group("failed", "Running"), 1);
}

#[test]
fn status_group_done_step() {
    assert_eq!(status_group("done", "Completed"), 2);
}

#[test]
fn status_group_step_status_failed() {
    assert_eq!(status_group("build", "Failed"), 1);
}

#[test]
fn status_group_step_status_completed() {
    assert_eq!(status_group("build", "Completed"), 2);
}

#[test]
fn sort_order_active_before_failed_before_done() {
    let mut pipelines = vec![
        PipelineSummary {
            id: "done-1".into(),
            name: "done-pipeline".into(),
            kind: "build".into(),
            step: "done".into(),
            step_status: "Completed".into(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
            namespace: String::new(),
        },
        PipelineSummary {
            id: "failed-1".into(),
            name: "failed-pipeline".into(),
            kind: "build".into(),
            step: "failed".into(),
            step_status: "Failed".into(),
            created_at_ms: 2000,
            updated_at_ms: 2000,
            namespace: String::new(),
        },
        PipelineSummary {
            id: "active-1".into(),
            name: "active-pipeline".into(),
            kind: "build".into(),
            step: "build".into(),
            step_status: "Running".into(),
            created_at_ms: 3000,
            updated_at_ms: 3000,
            namespace: String::new(),
        },
    ];

    pipelines.sort_by(|a, b| {
        let ga = status_group(&a.step, &a.step_status);
        let gb = status_group(&b.step, &b.step_status);
        ga.cmp(&gb).then(b.created_at_ms.cmp(&a.created_at_ms))
    });

    assert_eq!(pipelines[0].id, "active-1");
    assert_eq!(pipelines[1].id, "failed-1");
    assert_eq!(pipelines[2].id, "done-1");
}

#[test]
fn sort_order_most_recent_first_within_group() {
    let mut pipelines = vec![
        PipelineSummary {
            id: "old".into(),
            name: "old-pipeline".into(),
            kind: "build".into(),
            step: "build".into(),
            step_status: "Running".into(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
            namespace: String::new(),
        },
        PipelineSummary {
            id: "new".into(),
            name: "new-pipeline".into(),
            kind: "build".into(),
            step: "execute".into(),
            step_status: "Running".into(),
            created_at_ms: 5000,
            updated_at_ms: 5000,
            namespace: String::new(),
        },
    ];

    pipelines.sort_by(|a, b| {
        let ga = status_group(&a.step, &a.step_status);
        let gb = status_group(&b.step, &b.step_status);
        ga.cmp(&gb).then(b.created_at_ms.cmp(&a.created_at_ms))
    });

    assert_eq!(pipelines[0].id, "new");
    assert_eq!(pipelines[1].id, "old");
}

fn make_detail(name: &str, steps: Vec<StepRecordDetail>) -> PipelineDetail {
    PipelineDetail {
        id: "abc12345".into(),
        name: name.into(),
        kind: "pipeline".into(),
        step: "build".into(),
        step_status: "Running".into(),
        vars: HashMap::new(),
        workspace_path: None,
        session_id: None,
        error: None,
        steps,
        agents: vec![],
        namespace: String::new(),
    }
}

fn make_step(name: &str, outcome: &str, started: u64, finished: Option<u64>) -> StepRecordDetail {
    StepRecordDetail {
        name: name.into(),
        started_at_ms: started,
        finished_at_ms: finished,
        outcome: outcome.into(),
        detail: None,
        agent_id: None,
    }
}

fn output_string(buf: &[u8]) -> String {
    String::from_utf8_lossy(buf).to_string()
}

#[test]
fn step_progress_no_steps() {
    let detail = make_detail("test", vec![]);
    let mut tracker = StepTracker {
        printed_count: 0,
        printed_started: false,
    };
    let mut buf = Vec::new();
    print_step_progress(&detail, &mut tracker, false, &mut buf);
    assert_eq!(output_string(&buf), "");
    assert_eq!(tracker.printed_count, 0);
}

#[test]
fn step_progress_single_running() {
    let detail = make_detail("test", vec![make_step("plan", "running", 1000, None)]);
    let mut tracker = StepTracker {
        printed_count: 0,
        printed_started: false,
    };
    let mut buf = Vec::new();
    print_step_progress(&detail, &mut tracker, false, &mut buf);
    assert_eq!(output_string(&buf), "plan started\n");
    assert!(tracker.printed_started);
    assert_eq!(tracker.printed_count, 0);
}

#[test]
fn step_progress_single_completed() {
    let detail = make_detail(
        "test",
        vec![make_step("init", "completed", 1000, Some(1000))],
    );
    let mut tracker = StepTracker {
        printed_count: 0,
        printed_started: false,
    };
    let mut buf = Vec::new();
    print_step_progress(&detail, &mut tracker, false, &mut buf);
    assert_eq!(output_string(&buf), "init completed (0s)\n");
    assert_eq!(tracker.printed_count, 1);
}

#[test]
fn step_progress_skipped_running() {
    // Step goes directly from not-present to completed (fast step)
    let detail = make_detail(
        "test",
        vec![make_step("push", "completed", 5000, Some(5500))],
    );
    let mut tracker = StepTracker {
        printed_count: 0,
        printed_started: false,
    };
    let mut buf = Vec::new();
    print_step_progress(&detail, &mut tracker, false, &mut buf);
    // Should print only "completed", not "started" then "completed"
    assert_eq!(output_string(&buf), "push completed (0s)\n");
    assert_eq!(tracker.printed_count, 1);
}

#[test]
fn step_progress_multiple_steps_one_poll() {
    let detail = make_detail(
        "test",
        vec![
            make_step("init", "completed", 1000, Some(1000)),
            make_step("plan", "completed", 1000, Some(165000)),
        ],
    );
    let mut tracker = StepTracker {
        printed_count: 0,
        printed_started: false,
    };
    let mut buf = Vec::new();
    print_step_progress(&detail, &mut tracker, false, &mut buf);
    let out = output_string(&buf);
    assert!(out.contains("init completed (0s)\n"));
    assert!(out.contains("plan completed (2m 44s)\n"));
    assert_eq!(tracker.printed_count, 2);
}

#[test]
fn step_progress_failed_with_detail() {
    let mut step = make_step("implement", "failed", 1000, Some(453000));
    step.detail = Some("shell exit code: 2".into());
    let detail = make_detail("test", vec![step]);
    let mut tracker = StepTracker {
        printed_count: 0,
        printed_started: false,
    };
    let mut buf = Vec::new();
    print_step_progress(&detail, &mut tracker, false, &mut buf);
    assert_eq!(
        output_string(&buf),
        "implement failed (7m 32s) - shell exit code: 2\n"
    );
}

#[test]
fn step_progress_multi_pipeline_prefix() {
    let detail = make_detail(
        "auto-start-worker",
        vec![make_step("init", "completed", 1000, Some(1000))],
    );
    let mut tracker = StepTracker {
        printed_count: 0,
        printed_started: false,
    };
    let mut buf = Vec::new();
    print_step_progress(&detail, &mut tracker, true, &mut buf);
    assert_eq!(
        output_string(&buf),
        "[auto-start-worker] init completed (0s)\n"
    );
}

#[test]
fn step_progress_idempotent_repolling() {
    let detail = make_detail(
        "test",
        vec![make_step("init", "completed", 1000, Some(1000))],
    );
    let mut tracker = StepTracker {
        printed_count: 0,
        printed_started: false,
    };

    let mut buf = Vec::new();
    print_step_progress(&detail, &mut tracker, false, &mut buf);
    assert_eq!(output_string(&buf), "init completed (0s)\n");

    // Second poll with same state should produce no output
    let mut buf2 = Vec::new();
    print_step_progress(&detail, &mut tracker, false, &mut buf2);
    assert_eq!(output_string(&buf2), "");
}

#[test]
fn step_progress_running_then_completed() {
    // First poll: step is running
    let detail1 = make_detail("test", vec![make_step("plan", "running", 1000, None)]);
    let mut tracker = StepTracker {
        printed_count: 0,
        printed_started: false,
    };
    let mut buf = Vec::new();
    print_step_progress(&detail1, &mut tracker, false, &mut buf);
    assert_eq!(output_string(&buf), "plan started\n");

    // Second poll: step completed
    let detail2 = make_detail(
        "test",
        vec![make_step("plan", "completed", 1000, Some(165000))],
    );
    let mut buf2 = Vec::new();
    print_step_progress(&detail2, &mut tracker, false, &mut buf2);
    assert_eq!(output_string(&buf2), "plan completed (2m 44s)\n");
    assert_eq!(tracker.printed_count, 1);
}

#[test]
fn format_var_short_value_unchanged() {
    let value = "hello world";
    assert_eq!(format_var_value(value, 80), "hello world");
}

#[test]
fn format_var_long_value_truncated() {
    let value = "a".repeat(100);
    let result = format_var_value(&value, 80);
    assert_eq!(result.len(), 83); // 80 + "..."
    assert!(result.ends_with("..."));
    assert!(result.starts_with("aaaa"));
}

#[test]
fn format_var_newlines_escaped() {
    let value = "line1\nline2\nline3";
    assert_eq!(format_var_value(value, 80), "line1\\nline2\\nline3");
}

#[test]
fn format_var_newlines_and_truncation() {
    // Create a value with newlines that when escaped exceeds 80 chars
    // Each \n becomes \\n (2 chars), so 40 'a' + 41 newlines = 40 + 82 = 122 escaped chars
    let value = "a\n".repeat(40);
    let result = format_var_value(&value, 80);
    assert!(result.ends_with("..."));
    // The truncated part should be exactly 80 chars
    assert_eq!(result.chars().count(), 83); // 80 + "..."
}

fn make_summary(id: &str, name: &str, kind: &str, step: &str, status: &str) -> PipelineSummary {
    PipelineSummary {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        step: step.into(),
        step_status: status.into(),
        created_at_ms: 0,
        updated_at_ms: 0,
        namespace: String::new(),
    }
}

#[test]
fn list_empty() {
    let mut buf = Vec::new();
    format_pipeline_list(&mut buf, &[]);
    assert_eq!(output_string(&buf), "No pipelines\n");
}

#[test]
fn list_columns_fit_data() {
    let pipelines = vec![
        make_summary("abcdef123456", "my-build", "build", "plan", "Running"),
        make_summary("999999999999", "x", "fix", "implement", "Running"),
    ];
    let mut buf = Vec::new();
    format_pipeline_list(&mut buf, &pipelines);
    let out = output_string(&buf);
    let lines: Vec<&str> = out.lines().collect();

    // Header + 2 data rows
    assert_eq!(lines.len(), 3);

    // Columns should be tight to the widest value, not fixed
    // "my-build" (8) is wider than "NAME" (4), so NAME column = 8
    // "implement" (9) is wider than "STEP" (4), so STEP column = 9
    let header = lines[0];
    assert!(header.starts_with("ID"));
    assert!(header.contains("NAME"));
    assert!(header.contains("STATUS"));

    // Verify no excessive padding: "my-build" should be followed by minimal spacing
    // The row for "x" should have the name padded to match "my-build" width
    let row2 = lines[2];
    assert!(row2.contains("x       ")); // "x" padded to 8 chars (width of "my-build")
}

#[test]
fn list_with_project_column() {
    let mut p1 = make_summary("abcdef123456", "api-server", "build", "test", "Running");
    p1.namespace = "myproject".into();
    let mut p2 = make_summary("999999999999", "worker", "fix", "done", "Completed");
    p2.namespace = "other".into();
    let pipelines = vec![p1, p2];

    let mut buf = Vec::new();
    format_pipeline_list(&mut buf, &pipelines);
    let out = output_string(&buf);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("PROJECT"));
    // "myproject" (9) > "PROJECT" (7), so project column = 9
    assert!(lines[1].contains("myproject"));
    // "other" padded to 9 chars
    assert!(lines[2].contains("other    "));
}

#[test]
fn list_no_project_when_all_empty_namespace() {
    let pipelines = vec![make_summary(
        "abcdef123456",
        "build-a",
        "build",
        "plan",
        "Running",
    )];
    let mut buf = Vec::new();
    format_pipeline_list(&mut buf, &pipelines);
    let out = output_string(&buf);
    assert!(!out.contains("PROJECT"));
}
