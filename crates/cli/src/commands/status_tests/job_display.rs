use serial_test::serial;

use super::*;

// ── Name hiding (UUID/nonce names suppressed) ───────────────────────

#[test]
#[serial]
fn active_job_shows_kind_not_name() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![make_job(
            "abcd1234-0000-0000-0000",
            "abcd1234-0000-0000-0000",
            "build",
            "check",
            "running",
        )],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("build"),
        "output should contain job kind 'build':\n{output}"
    );
    assert!(
        output.contains("check"),
        "output should contain step name 'check':\n{output}"
    );
    assert!(
        !output.contains("abcd1234-0000"),
        "output should not contain the UUID name:\n{output}"
    );
}

#[test]
#[serial]
fn active_job_hides_nonce_only_name() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![make_job(
            "abcd1234-0000-0000-0000",
            "abcd1234",
            "build",
            "check",
            "running",
        )],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("build"),
        "output should contain job kind 'build':\n{output}"
    );
    assert!(
        output.contains("check"),
        "output should contain step name 'check':\n{output}"
    );
    let nonce_count = output.matches("abcd1234").count();
    assert_eq!(
        nonce_count, 1,
        "nonce 'abcd1234' should appear exactly once (as truncated ID), not twice:\n{output}"
    );
}

#[test]
#[serial]
fn escalated_job_hides_name_when_same_as_id() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![{
            let mut j = make_job(
                "efgh5678-0000-0000-0000",
                "efgh5678-0000-0000-0000",
                "deploy",
                "test",
                "waiting",
            );
            j.waiting_reason = Some("gate check failed".to_string());
            j
        }],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("deploy"),
        "output should contain job kind 'deploy':\n{output}"
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
        !output.contains("efgh5678-0000"),
        "output should not contain the UUID name:\n{output}"
    );
}

#[test]
#[serial]
fn orphaned_job_hides_name_when_same_as_id() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![make_job(
            "ijkl9012-0000-0000-0000",
            "ijkl9012-0000-0000-0000",
            "ci",
            "lint",
            "running",
        )],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("ci"),
        "output should contain job kind 'ci':\n{output}"
    );
    assert!(
        output.contains("lint"),
        "output should contain step name 'lint':\n{output}"
    );
    assert!(
        !output.contains("ijkl9012-0000"),
        "output should not contain the UUID name:\n{output}"
    );
}

// ── Friendly name display ───────────────────────────────────────────

#[test]
#[serial]
fn active_job_shows_friendly_name() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![make_job(
            "abcd1234-0000-0000-0000",
            "fix-login-button-abcd1234",
            "build",
            "check",
            "running",
        )],
        escalated_jobs: vec![],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("build"),
        "output should contain job kind 'build':\n{output}"
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
fn escalated_job_shows_friendly_name() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![{
            let mut j = make_job(
                "efgh5678-0000-0000-0000",
                "deploy-staging-efgh5678",
                "deploy",
                "test",
                "waiting",
            );
            j.waiting_reason = Some("gate check failed".to_string());
            j
        }],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("deploy"),
        "output should contain job kind 'deploy':\n{output}"
    );
    assert!(
        output.contains("deploy-staging-efgh5678"),
        "output should contain friendly name:\n{output}"
    );
}

#[test]
#[serial]
fn orphaned_job_shows_friendly_name() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![],
        orphaned_jobs: vec![make_job(
            "ijkl9012-0000-0000-0000",
            "ci-main-branch-ijkl9012",
            "ci",
            "lint",
            "running",
        )],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("ci"),
        "output should contain job kind 'ci':\n{output}"
    );
    assert!(
        output.contains("ci-main-branch-ijkl9012"),
        "output should contain friendly name:\n{output}"
    );
}

// ── Escalation source labels ────────────────────────────────────────

#[test]
#[serial]
fn escalated_job_shows_source_label() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![{
            let mut j = make_job(
                "efgh5678-0000-0000-0000",
                "deploy-staging-efgh5678",
                "deploy",
                "test",
                "waiting",
            );
            j.waiting_reason = Some("Agent is idle".to_string());
            j.escalate_source = Some("idle".to_string());
            j
        }],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    assert!(
        output.contains("[idle]"),
        "output should contain source label '[idle]':\n{output}"
    );
}

#[test]
#[serial]
fn escalated_job_no_source_label_when_none() {
    set_no_color();

    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![{
            let mut j = make_job(
                "efgh5678-0000-0000-0000",
                "deploy-staging-efgh5678",
                "deploy",
                "test",
                "waiting",
            );
            j.waiting_reason = Some("gate check failed".to_string());
            j
        }],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    assert!(
        !output.contains('['),
        "output should not contain bracket source label when source is None:\n{output}"
    );
}

// ── Reason truncation ───────────────────────────────────────────────

#[test]
#[serial]
fn escalated_job_truncates_long_reason() {
    set_no_color();

    let long_reason = "e".repeat(200);
    let ns = NamespaceStatus {
        namespace: "myproject".to_string(),
        active_jobs: vec![],
        escalated_jobs: vec![{
            let mut j = make_job("efgh5678", "efgh5678", "deploy", "test", "Waiting");
            j.waiting_reason = Some(long_reason.clone());
            j
        }],
        orphaned_jobs: vec![],
        workers: vec![],
        queues: vec![],
        active_agents: vec![],
        pending_decisions: 0,
    };

    let output = format_text(30, &[ns], None);

    assert!(
        !output.contains(&long_reason),
        "output should not contain the full long reason:\n{output}"
    );
    assert!(
        output.contains("..."),
        "output should contain truncation indicator '...':\n{output}"
    );
}
