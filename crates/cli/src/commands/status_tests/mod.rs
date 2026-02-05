use oj_daemon::NamespaceStatus;

mod formatting;
mod frame;
mod job_display;
mod layout;

/// Disable color output for deterministic test assertions.
pub(super) fn setup_no_color() {
    std::env::set_var("NO_COLOR", "1");
    std::env::remove_var("COLOR");
}

/// Create a namespace with one active job (for filter tests).
pub(super) fn make_ns(name: &str) -> NamespaceStatus {
    let mut entry = job_entry("abc12345", "job", "compile");
    entry.name = "build".to_string();
    entry.elapsed_ms = 5000;
    let mut ns = empty_ns(name);
    ns.active_jobs.push(entry);
    ns
}

/// Create a minimal job entry with sensible defaults.
///
/// `name` defaults to `id`; override fields as needed.
pub(super) fn job_entry(id: &str, kind: &str, step: &str) -> oj_daemon::JobStatusEntry {
    oj_daemon::JobStatusEntry {
        id: id.to_string(),
        name: id.to_string(),
        kind: kind.to_string(),
        step: step.to_string(),
        step_status: "running".to_string(),
        elapsed_ms: 60_000,
        last_activity_ms: 0,
        waiting_reason: None,
        escalate_source: None,
    }
}

/// Create an empty namespace.
pub(super) fn empty_ns(name: &str) -> NamespaceStatus {
    NamespaceStatus {
        namespace: name.to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    }
}
