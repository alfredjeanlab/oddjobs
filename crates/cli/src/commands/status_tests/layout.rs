use serial_test::serial;

use super::*;

// ── Column order & alignment ────────────────────────────────────────

#[test]
#[serial]
fn column_order_is_id_name_kindstep_status_elapsed() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![{
            let mut j = make_job(
                "abcd1234-0000-0000-0000",
                "fix-login-abcd1234",
                "build",
                "check",
                "running",
            );
            j.elapsed_ms = 420_000;
            j
        }],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    let line = output
        .lines()
        .find(|l| l.contains("abcd1234"))
        .expect("should find job row");

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
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![
            make_job(
                "aaaa1111-0000",
                "short-aaaa1111",
                "build",
                "check",
                "running",
            ),
            {
                let mut j = make_job(
                    "bbbb2222-0000",
                    "much-longer-name-bbbb2222",
                    "deploy",
                    "implement",
                    "waiting",
                );
                j.elapsed_ms = 120_000;
                j
            },
        ],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    let lines: Vec<&str> = output
        .lines()
        .filter(|l| l.contains("aaaa1111") || l.contains("bbbb2222"))
        .collect();
    assert_eq!(lines.len(), 2, "should find exactly 2 job rows");

    let ks_pos_0 = lines[0].find("build/check").unwrap();
    let ks_pos_1 = lines[1].find("deploy/implement").unwrap();
    assert_eq!(
        ks_pos_0, ks_pos_1,
        "kind/step columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );

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
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![
            make_job(
                "aaaa1111-0000-0000-0000",
                "aaaa1111-0000-0000-0000",
                "build",
                "check",
                "running",
            ),
            {
                let mut j = make_job(
                    "bbbb2222-0000-0000-0000",
                    "build",
                    "build",
                    "test",
                    "running",
                );
                j.elapsed_ms = 120_000;
                j
            },
        ],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    let line = output
        .lines()
        .find(|l| l.contains("aaaa1111"))
        .expect("should find first job row");

    let id_end = line.find("aaaa1111").unwrap() + "aaaa1111".len();
    let ks_start = line.find("build/check").unwrap();
    assert_eq!(
        ks_start - id_end,
        2,
        "kind/step should follow id with just 2-space separator when names are hidden:\n  {line}"
    );
}

// ── Worker column alignment ─────────────────────────────────────────

#[test]
#[serial]
fn worker_columns_are_aligned_across_rows() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
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
                status: "stopped".to_string(),
                active: 0,
                concurrency: 2,
                updated_at_ms: 0,
            },
        ],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    let lines: Vec<&str> = output.lines().filter(|l| l.contains("active")).collect();
    assert_eq!(lines.len(), 2, "should find exactly 2 worker rows");

    let st_pos_0 = lines[0].find("on").unwrap();
    let st_pos_1 = lines[1].find("off").unwrap();
    assert_eq!(
        st_pos_0, st_pos_1,
        "status columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );
}

#[test]
#[serial]
fn worker_shows_full_at_max_concurrency() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![oj_daemon::WorkerSummary {
            name: "busy".to_string(),
            namespace: "myproject".to_string(),
            queue: "default".to_string(),
            status: "running".to_string(),
            active: 3,
            concurrency: 3,
            updated_at_ms: 0,
        }],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);
    let line = output.lines().find(|l| l.contains("busy")).unwrap();
    assert!(
        line.contains("full"),
        "worker at max concurrency should show 'full': {line}"
    );
    assert!(
        !line.contains("on"),
        "worker at max concurrency should show 'full' not 'on': {line}"
    );
}

// ── Queue column alignment ──────────────────────────────────────────

#[test]
#[serial]
fn queue_columns_are_aligned_across_rows() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
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
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    let lines: Vec<&str> = output.lines().filter(|l| l.contains("pending")).collect();
    assert_eq!(lines.len(), 2, "should find exactly 2 queue rows");

    let num_start_0 = lines[0].find(|c: char| c.is_ascii_digit()).unwrap();
    let num_start_1 = lines[1].find(|c: char| c.is_ascii_digit()).unwrap();
    assert_eq!(
        num_start_0, num_start_1,
        "count columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );
}

// ── Agent column alignment ──────────────────────────────────────────

#[test]
#[serial]
fn agent_columns_are_aligned_across_rows() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![
            oj_daemon::AgentStatusEntry {
                agent_name: "coder".to_string(),
                command_name: "build".to_string(),
                agent_id: "agent-01".to_string(),
                status: "running".to_string(),
            },
            oj_daemon::AgentStatusEntry {
                agent_name: "long-agent-name".to_string(),
                command_name: "deploy".to_string(),
                agent_id: "agent-02".to_string(),
                status: "idle".to_string(),
            },
        ],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    let lines: Vec<&str> = output.lines().filter(|l| l.contains("agent-")).collect();
    assert_eq!(lines.len(), 2, "should find exactly 2 agent rows");

    let id_pos_0 = lines[0].find("agent-01").unwrap();
    let id_pos_1 = lines[1].find("agent-02").unwrap();
    assert_eq!(
        id_pos_0, id_pos_1,
        "agent_id columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );

    let st_pos_0 = lines[0].find("running").unwrap();
    let st_pos_1 = lines[1].find("idle").unwrap();
    assert_eq!(
        st_pos_0, st_pos_1,
        "status columns should be aligned:\n  {}\n  {}",
        lines[0], lines[1]
    );
}

// ── Namespace visibility ────────────────────────────────────────────

#[test]
#[serial]
fn namespace_with_only_empty_queues_is_hidden() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "empty-project".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![oj_daemon::QueueStatus {
            name: "tasks".to_string(),
            pending: 0,
            active: 0,
            dead: 0,
        }],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(60, &[ns], None);

    assert!(
        !output.contains("empty-project"),
        "namespace with only empty queues should be hidden:\n{output}"
    );
    assert_eq!(output, "oj daemon: running 1m\n");
}

#[test]
#[serial]
fn namespace_with_non_empty_queue_is_shown() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "active-project".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![oj_daemon::QueueStatus {
            name: "tasks".to_string(),
            pending: 1,
            active: 0,
            dead: 0,
        }],
        active_agents: vec![],
        pending_decisions: 0,
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

// ── filter_namespaces ───────────────────────────────────────────────

#[test]
fn filter_namespaces_none_returns_all() {
    let namespaces = vec![make_ns("alpha"), make_ns("beta"), make_ns("gamma")];
    let filtered = filter_namespaces(namespaces, None);
    assert_eq!(filtered.len(), 3);
}

#[test]
fn filter_namespaces_matches_project() {
    let namespaces = vec![make_ns("alpha"), make_ns("beta"), make_ns("gamma")];
    let filtered = filter_namespaces(namespaces, Some("beta"));
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].namespace, "beta");
}

#[test]
fn filter_namespaces_no_match_returns_empty() {
    let namespaces = vec![make_ns("alpha"), make_ns("beta")];
    let filtered = filter_namespaces(namespaces, Some("nonexistent"));
    assert!(filtered.is_empty());
}

#[test]
#[serial]
fn project_filter_restricts_text_output() {
    set_no_color();

    let namespaces = vec![make_ns("alpha"), make_ns("beta")];
    let filtered = filter_namespaces(namespaces, Some("alpha"));
    let output = format_text(60, &filtered, None);

    assert!(
        output.contains("alpha"),
        "output should contain the filtered project:\n{output}"
    );
    assert!(
        !output.contains("beta"),
        "output should not contain other projects:\n{output}"
    );
}

// ── Sorting ─────────────────────────────────────────────────────────

#[test]
#[serial]
fn workers_sorted_alphabetically() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![
            oj_daemon::WorkerSummary {
                name: "zebra".to_string(),
                namespace: "myproject".to_string(),
                queue: "default".to_string(),
                status: "running".to_string(),
                active: 1,
                concurrency: 2,
                updated_at_ms: 0,
            },
            oj_daemon::WorkerSummary {
                name: "alpha".to_string(),
                namespace: "myproject".to_string(),
                queue: "default".to_string(),
                status: "running".to_string(),
                active: 0,
                concurrency: 2,
                updated_at_ms: 0,
            },
            oj_daemon::WorkerSummary {
                name: "mid".to_string(),
                namespace: "myproject".to_string(),
                queue: "default".to_string(),
                status: "idle".to_string(),
                active: 0,
                concurrency: 1,
                updated_at_ms: 0,
            },
        ],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    let worker_lines: Vec<&str> = output.lines().filter(|l| l.contains("active")).collect();
    assert_eq!(worker_lines.len(), 3, "should find 3 worker rows");

    let alpha_pos = output.find("alpha").unwrap();
    let mid_pos = output.find("mid").unwrap();
    let zebra_pos = output.find("zebra").unwrap();
    assert!(
        alpha_pos < mid_pos && mid_pos < zebra_pos,
        "workers should be sorted alphabetically: alpha < mid < zebra\n{output}"
    );
}

#[test]
#[serial]
fn jobs_sorted_by_most_recent_activity() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![
            {
                let mut j = make_job("oldest-0000", "oldest-0000", "build", "check", "running");
                j.elapsed_ms = 300_000;
                j.last_activity_ms = 1000;
                j
            },
            {
                let mut j = make_job("newest-0000", "newest-0000", "build", "test", "running");
                j.last_activity_ms = 3000;
                j
            },
            {
                let mut j = make_job("middle-0000", "middle-0000", "build", "lint", "running");
                j.elapsed_ms = 120_000;
                j.last_activity_ms = 2000;
                j
            },
        ],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    let newest_pos = output.find("newest").unwrap();
    let middle_pos = output.find("middle").unwrap();
    let oldest_pos = output.find("oldest").unwrap();
    assert!(
        newest_pos < middle_pos && middle_pos < oldest_pos,
        "jobs should be sorted by most recent activity first\n{output}"
    );
}
