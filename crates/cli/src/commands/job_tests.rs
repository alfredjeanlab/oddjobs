// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::super::job_wait::{print_step_progress, StepTracker};
use super::{
    format_job_list, format_var_value, is_var_truncated, parse_duration, sorted_vars,
    var_namespace_priority,
};
use oj_daemon::{JobDetail, JobSummary, StepRecordDetail};
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
fn sort_order_most_recently_updated_first() {
    let mut jobs = vec![
        JobSummary {
            id: "done-1".into(),
            name: "done-job".into(),
            kind: "build".into(),
            step: "done".into(),
            step_status: "completed".into(),
            created_at_ms: 1000,
            updated_at_ms: 5000,
            namespace: String::new(),
            retry_count: 0,
        },
        JobSummary {
            id: "failed-1".into(),
            name: "failed-job".into(),
            kind: "build".into(),
            step: "failed".into(),
            step_status: "failed".into(),
            created_at_ms: 2000,
            updated_at_ms: 2000,
            namespace: String::new(),
            retry_count: 0,
        },
        JobSummary {
            id: "active-1".into(),
            name: "active-job".into(),
            kind: "build".into(),
            step: "build".into(),
            step_status: "running".into(),
            created_at_ms: 3000,
            updated_at_ms: 3000,
            namespace: String::new(),
            retry_count: 0,
        },
    ];

    jobs.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));

    assert_eq!(jobs[0].id, "done-1"); // updated_at 5000
    assert_eq!(jobs[1].id, "active-1"); // updated_at 3000
    assert_eq!(jobs[2].id, "failed-1"); // updated_at 2000
}

#[test]
fn sort_order_most_recent_updated_first_within_same_status() {
    let mut jobs = vec![
        JobSummary {
            id: "old".into(),
            name: "old-job".into(),
            kind: "build".into(),
            step: "build".into(),
            step_status: "running".into(),
            created_at_ms: 1000,
            updated_at_ms: 1000,
            namespace: String::new(),
            retry_count: 0,
        },
        JobSummary {
            id: "new".into(),
            name: "new-job".into(),
            kind: "build".into(),
            step: "execute".into(),
            step_status: "running".into(),
            created_at_ms: 5000,
            updated_at_ms: 5000,
            namespace: String::new(),
            retry_count: 0,
        },
    ];

    jobs.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));

    assert_eq!(jobs[0].id, "new");
    assert_eq!(jobs[1].id, "old");
}

fn make_detail(name: &str, steps: Vec<StepRecordDetail>) -> JobDetail {
    JobDetail {
        id: "abc12345".into(),
        name: name.into(),
        kind: "job".into(),
        step: "build".into(),
        step_status: "running".into(),
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
        agent_name: None,
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
fn step_progress_multi_job_prefix() {
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

#[test]
fn is_var_truncated_short_value() {
    let value = "hello world";
    assert!(!is_var_truncated(value, 80));
}

#[test]
fn is_var_truncated_long_value() {
    let value = "a".repeat(100);
    assert!(is_var_truncated(&value, 80));
}

#[test]
fn is_var_truncated_exact_length() {
    let value = "a".repeat(80);
    assert!(!is_var_truncated(&value, 80));
}

#[test]
fn is_var_truncated_with_newlines() {
    // Each \n becomes \\n (2 chars), so 40 'a' + 41 newlines = 40 + 82 = 122 escaped chars
    let value = "a\n".repeat(40);
    assert!(is_var_truncated(&value, 80));
}

fn make_summary_ns(
    id: &str,
    name: &str,
    kind: &str,
    step: &str,
    status: &str,
    namespace: &str,
) -> JobSummary {
    JobSummary {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        step: step.into(),
        step_status: status.into(),
        created_at_ms: 0,
        updated_at_ms: 0,
        namespace: namespace.into(),
        retry_count: 0,
    }
}

fn make_summary(id: &str, name: &str, kind: &str, step: &str, status: &str) -> JobSummary {
    JobSummary {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        step: step.into(),
        step_status: status.into(),
        created_at_ms: 0,
        updated_at_ms: 0,
        namespace: String::new(),
        retry_count: 0,
    }
}

#[test]
fn list_empty() {
    let mut buf = Vec::new();
    format_job_list(&mut buf, &[]);
    assert_eq!(output_string(&buf), "No jobs\n");
}

#[test]
fn list_columns_fit_data() {
    let jobs = vec![
        make_summary("abcdef123456", "my-build", "build", "plan", "running"),
        make_summary("999999999999", "x", "fix", "implement", "running"),
    ];
    let mut buf = Vec::new();
    format_job_list(&mut buf, &jobs);
    let out = output_string(&buf);
    let lines: Vec<&str> = out.lines().collect();

    // Header + 2 data rows
    assert_eq!(lines.len(), 3);

    // Columns should be tight to the widest value, not fixed
    // "my-build" (8) is wider than "NAME" (4), so NAME column = 8
    // "implement" (9) is wider than "STEP" (4), so STEP column = 9
    let header = lines[0];
    assert!(header.contains("ID"));
    assert!(header.contains("NAME"));
    assert!(header.contains("STATUS"));

    // Verify no excessive padding: "my-build" should be followed by minimal spacing
    // The row for "x" should have the name padded to match "my-build" width
    let row2 = lines[2];
    assert!(row2.contains("x       ")); // "x" padded to 8 chars (width of "my-build")
}

#[test]
fn list_with_project_column() {
    let mut p1 = make_summary("abcdef123456", "api-server", "build", "test", "running");
    p1.namespace = "myproject".into();
    let mut p2 = make_summary("999999999999", "worker", "fix", "done", "completed");
    p2.namespace = "other".into();
    let jobs = vec![p1, p2];

    let mut buf = Vec::new();
    format_job_list(&mut buf, &jobs);
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
fn list_mixed_namespace_shows_no_project_for_empty() {
    let mut p1 = make_summary("abcdef123456", "api-server", "build", "test", "running");
    p1.namespace = "myproject".into();
    let p2 = make_summary("999999999999", "worker", "fix", "done", "completed");
    // p2 has empty namespace
    let jobs = vec![p1, p2];

    let mut buf = Vec::new();
    format_job_list(&mut buf, &jobs);
    let out = output_string(&buf);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("PROJECT"));
    assert!(lines[1].contains("myproject"));
    assert!(lines[2].contains("(no project)"));
}

#[test]
fn list_no_project_when_all_empty_namespace() {
    let jobs = vec![make_summary(
        "abcdef123456",
        "build-a",
        "build",
        "plan",
        "running",
    )];
    let mut buf = Vec::new();
    format_job_list(&mut buf, &jobs);
    let out = output_string(&buf);
    assert!(!out.contains("PROJECT"));
}

#[test]
fn list_no_retries_column_when_all_zero() {
    let jobs = vec![make_summary(
        "abcdef123456",
        "build-a",
        "build",
        "plan",
        "running",
    )];
    let mut buf = Vec::new();
    format_job_list(&mut buf, &jobs);
    let out = output_string(&buf);
    assert!(!out.contains("RETRIES"));
}

#[test]
fn list_retries_column_shown_when_nonzero() {
    let mut p1 = make_summary("abcdef123456", "build-a", "build", "plan", "running");
    p1.retry_count = 3;
    let p2 = make_summary("999999999999", "build-b", "build", "test", "running");
    let jobs = vec![p1, p2];
    let mut buf = Vec::new();
    format_job_list(&mut buf, &jobs);
    let out = output_string(&buf);
    let lines: Vec<&str> = out.lines().collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("RETRIES"));
    assert!(lines[1].contains("3"));
    assert!(lines[2].contains("0"));
}

// ── Project filter tests ─────────────────────────────────────────────────

#[test]
fn project_filter_retains_matching_namespace() {
    let mut jobs = vec![
        make_summary_ns("aaa", "build-wok", "build", "plan", "running", "wok"),
        make_summary_ns("bbb", "build-bar", "build", "test", "running", "bar"),
        make_summary_ns("ccc", "build-wok2", "build", "done", "completed", "wok"),
    ];

    // Simulate: --project wok
    let project_filter: Option<&str> = Some("wok");
    if let Some(proj) = project_filter {
        jobs.retain(|p| p.namespace == proj);
    }

    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0].id, "aaa");
    assert_eq!(jobs[1].id, "ccc");
}

#[test]
fn project_filter_none_retains_all() {
    let mut jobs = vec![
        make_summary_ns("aaa", "build-wok", "build", "plan", "running", "wok"),
        make_summary_ns("bbb", "build-bar", "build", "test", "running", "bar"),
        make_summary_ns("ccc", "build-wok2", "build", "done", "completed", "wok"),
    ];

    // Simulate: no --project flag (OJ_NAMESPACE should NOT filter)
    let project_filter: Option<&str> = None;
    if let Some(proj) = project_filter {
        jobs.retain(|p| p.namespace == proj);
    }

    assert_eq!(
        jobs.len(),
        3,
        "all jobs should be retained when no --project flag"
    );
}

#[test]
fn project_filter_no_match_returns_empty() {
    let mut jobs = vec![
        make_summary_ns("aaa", "build-wok", "build", "plan", "running", "wok"),
        make_summary_ns("bbb", "build-bar", "build", "test", "running", "bar"),
    ];

    let project_filter: Option<&str> = Some("nonexistent");
    if let Some(proj) = project_filter {
        jobs.retain(|p| p.namespace == proj);
    }

    assert!(jobs.is_empty());
}

// ── Variable sorting tests ─────────────────────────────────────────────────

#[test]
fn var_namespace_priority_invoke_first() {
    assert_eq!(var_namespace_priority("invoke.item"), 0);
    assert_eq!(var_namespace_priority("invoke.foo.bar"), 0);
}

#[test]
fn var_namespace_priority_workspace_second() {
    assert_eq!(var_namespace_priority("workspace.path"), 1);
    assert_eq!(var_namespace_priority("workspace.name"), 1);
}

#[test]
fn var_namespace_priority_local_third() {
    assert_eq!(var_namespace_priority("local.temp"), 2);
    assert_eq!(var_namespace_priority("local.result"), 2);
}

#[test]
fn var_namespace_priority_var_fourth() {
    assert_eq!(var_namespace_priority("var.input"), 3);
    assert_eq!(var_namespace_priority("var.output"), 3);
}

#[test]
fn var_namespace_priority_other_last() {
    assert_eq!(var_namespace_priority("custom.value"), 4);
    assert_eq!(var_namespace_priority("unknown"), 4);
    assert_eq!(var_namespace_priority("result"), 4);
}

#[test]
fn sorted_vars_groups_by_namespace() {
    let mut vars = HashMap::new();
    vars.insert("var.output".to_string(), "1".to_string());
    vars.insert("invoke.item".to_string(), "2".to_string());
    vars.insert("local.temp".to_string(), "3".to_string());
    vars.insert("workspace.path".to_string(), "4".to_string());

    let sorted = sorted_vars(&vars);
    let keys: Vec<&str> = sorted.iter().map(|(k, _)| k.as_str()).collect();

    assert_eq!(
        keys,
        vec!["invoke.item", "workspace.path", "local.temp", "var.output"]
    );
}

#[test]
fn sorted_vars_alphabetical_within_namespace() {
    let mut vars = HashMap::new();
    vars.insert("var.zebra".to_string(), "1".to_string());
    vars.insert("var.apple".to_string(), "2".to_string());
    vars.insert("var.mango".to_string(), "3".to_string());

    let sorted = sorted_vars(&vars);
    let keys: Vec<&str> = sorted.iter().map(|(k, _)| k.as_str()).collect();

    assert_eq!(keys, vec!["var.apple", "var.mango", "var.zebra"]);
}

#[test]
fn sorted_vars_mixed_namespaces_alphabetical() {
    let mut vars = HashMap::new();
    // Add vars in random order
    vars.insert("var.c".to_string(), "1".to_string());
    vars.insert("invoke.b".to_string(), "2".to_string());
    vars.insert("invoke.a".to_string(), "3".to_string());
    vars.insert("local.z".to_string(), "4".to_string());
    vars.insert("local.a".to_string(), "5".to_string());
    vars.insert("workspace.x".to_string(), "6".to_string());
    vars.insert("var.a".to_string(), "7".to_string());
    vars.insert("other.value".to_string(), "8".to_string());

    let sorted = sorted_vars(&vars);
    let keys: Vec<&str> = sorted.iter().map(|(k, _)| k.as_str()).collect();

    // Expected order: invoke.* (alphabetical), workspace.*, local.* (alphabetical), var.* (alphabetical), other.*
    assert_eq!(
        keys,
        vec![
            "invoke.a",
            "invoke.b",
            "workspace.x",
            "local.a",
            "local.z",
            "var.a",
            "var.c",
            "other.value"
        ]
    );
}

#[test]
fn sorted_vars_empty() {
    let vars: HashMap<String, String> = HashMap::new();
    let sorted = sorted_vars(&vars);
    assert!(sorted.is_empty());
}
