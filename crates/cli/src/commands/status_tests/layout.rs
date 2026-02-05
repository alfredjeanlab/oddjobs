use serial_test::serial;

use super::super::{filter_namespaces, format_text};
use super::{empty_ns, job_entry, make_ns, setup_no_color};

// ── job column order ────────────────────────────────────────────────

#[test]
#[serial]
fn column_order_is_id_name_kindstep_status_elapsed() {
    setup_no_color();

    let mut entry = job_entry("abcd1234-0000-0000-0000", "build", "check");
    entry.name = "fix-login-abcd1234".to_string();
    entry.elapsed_ms = 420_000;
    let mut ns = empty_ns("myproject");
    ns.active_jobs.push(entry);

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

// ── column alignment ────────────────────────────────────────────────

#[test]
#[serial]
fn columns_are_aligned_across_rows() {
    setup_no_color();

    let mut entry1 = job_entry("aaaa1111-0000", "build", "check");
    entry1.name = "short-aaaa1111".to_string();
    let mut entry2 = job_entry("bbbb2222-0000", "deploy", "implement");
    entry2.name = "much-longer-name-bbbb2222".to_string();
    entry2.step_status = "waiting".to_string();
    entry2.elapsed_ms = 120_000;
    let mut ns = empty_ns("myproject");
    ns.active_jobs.push(entry1);
    ns.active_jobs.push(entry2);

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
    setup_no_color();

    // name == id → hidden
    let entry1 = job_entry("aaaa1111-0000-0000-0000", "build", "check");
    // name == kind → hidden
    let mut entry2 = job_entry("bbbb2222-0000-0000-0000", "build", "test");
    entry2.name = "build".to_string();
    entry2.elapsed_ms = 120_000;
    let mut ns = empty_ns("myproject");
    ns.active_jobs.push(entry1);
    ns.active_jobs.push(entry2);

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

// ── worker layout ───────────────────────────────────────────────────

#[test]
#[serial]
fn worker_columns_are_aligned_across_rows() {
    setup_no_color();

    let mut ns = empty_ns("myproject");
    ns.workers.push(oj_daemon::WorkerSummary {
        name: "a".to_string(),
        namespace: "myproject".to_string(),
        queue: "default".to_string(),
        status: "running".to_string(),
        active: 1,
        concurrency: 4,
        updated_at_ms: 0,
    });
    ns.workers.push(oj_daemon::WorkerSummary {
        name: "long-worker-name".to_string(),
        namespace: "myproject".to_string(),
        queue: "default".to_string(),
        status: "stopped".to_string(),
        active: 0,
        concurrency: 2,
        updated_at_ms: 0,
    });

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
    setup_no_color();

    let mut ns = empty_ns("myproject");
    ns.workers.push(oj_daemon::WorkerSummary {
        name: "busy".to_string(),
        namespace: "myproject".to_string(),
        queue: "default".to_string(),
        status: "running".to_string(),
        active: 3,
        concurrency: 3,
        updated_at_ms: 0,
    });

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

// ── queue layout ────────────────────────────────────────────────────

#[test]
#[serial]
fn queue_columns_are_aligned_across_rows() {
    setup_no_color();

    let mut ns = empty_ns("myproject");
    ns.queues.push(oj_daemon::QueueStatus {
        name: "tasks".to_string(),
        pending: 3,
        active: 1,
        dead: 0,
    });
    ns.queues.push(oj_daemon::QueueStatus {
        name: "long-queue-name".to_string(),
        pending: 12,
        active: 2,
        dead: 1,
    });

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

// ── agent layout ────────────────────────────────────────────────────

#[test]
#[serial]
fn agent_columns_are_aligned_across_rows() {
    setup_no_color();

    let mut ns = empty_ns("myproject");
    ns.active_agents.push(oj_daemon::AgentStatusEntry {
        agent_name: "coder".to_string(),
        command_name: "build".to_string(),
        agent_id: "agent-01".to_string(),
        status: "running".to_string(),
    });
    ns.active_agents.push(oj_daemon::AgentStatusEntry {
        agent_name: "long-agent-name".to_string(),
        command_name: "deploy".to_string(),
        agent_id: "agent-02".to_string(),
        status: "idle".to_string(),
    });

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
    setup_no_color();

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

// ── sorting ─────────────────────────────────────────────────────────

#[test]
#[serial]
fn workers_sorted_alphabetically() {
    setup_no_color();

    let mut ns = empty_ns("myproject");
    for (name, active) in [("zebra", 1usize), ("alpha", 0), ("mid", 0)] {
        ns.workers.push(oj_daemon::WorkerSummary {
            name: name.to_string(),
            namespace: "myproject".to_string(),
            queue: "default".to_string(),
            status: if active > 0 { "running" } else { "idle" }.to_string(),
            active,
            concurrency: 2,
            updated_at_ms: 0,
        });
    }

    let output = format_text(30, &[ns], None);

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
    setup_no_color();

    let mut ns = empty_ns("myproject");
    for (id, step, elapsed, activity) in [
        ("oldest-0000", "check", 300_000u64, 1000u64),
        ("newest-0000", "test", 60_000, 3000),
        ("middle-0000", "lint", 120_000, 2000),
    ] {
        let mut entry = job_entry(id, "build", step);
        entry.elapsed_ms = elapsed;
        entry.last_activity_ms = activity;
        ns.active_jobs.push(entry);
    }

    let output = format_text(30, &[ns], None);

    let newest_pos = output.find("newest").unwrap();
    let middle_pos = output.find("middle").unwrap();
    let oldest_pos = output.find("oldest").unwrap();
    assert!(
        newest_pos < middle_pos && middle_pos < oldest_pos,
        "jobs should be sorted by most recent activity first\n{output}"
    );
}
