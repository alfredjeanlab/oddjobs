use oj_daemon::NamespaceStatus;

use super::{
    filter_namespaces, format_duration, format_text, friendly_name_label, render_frame,
    truncate_reason, CLEAR_TO_END, CLEAR_TO_EOL, CURSOR_HOME,
};

mod header;
mod helpers;
mod job_display;
mod layout;
mod render_frame_tests;

/// Create a minimal `NamespaceStatus` with one active job for filter tests.
fn make_ns(name: &str) -> NamespaceStatus {
    NamespaceStatus {
        namespace: name.to_string(),
        active_jobs: vec![make_job("abc12345", "build", "job", "compile", "running")],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    }
}

/// Create a `JobStatusEntry` with common defaults.
fn make_job(
    id: &str,
    name: &str,
    kind: &str,
    step: &str,
    step_status: &str,
) -> oj_daemon::JobStatusEntry {
    oj_daemon::JobStatusEntry {
        id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        step: step.to_string(),
        step_status: step_status.to_string(),
        elapsed_ms: 60_000,
        last_activity_ms: 0,
        waiting_reason: None,
        escalate_source: None,
    }
}

/// Set up environment for NO_COLOR text tests.
fn set_no_color() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");
}
