//! Namespace isolation specs
//!
//! Verify that projects with different namespaces operate independently
//! even when sharing the same daemon.

use crate::prelude::*;
use std::path::PathBuf;

/// Runbook with a simple shell job and queue.
const SIMPLE_JOB_RUNBOOK: &str = r#"
[queue.tasks]
type = "persisted"
vars = ["msg"]

[worker.runner]
source = { queue = "tasks" }
handler = { job = "process" }
concurrency = 1

[job.process]
vars = ["msg"]

[[job.process.step]]
name = "work"
run = "echo ${var.msg}"
"#;

/// Runbook with a command that runs a job echoing OJ_NAMESPACE.
const NAMESPACE_ECHO_RUNBOOK: &str = r#"
[command.check]
args = "<expected>"
run = { job = "check_namespace" }

[job.check_namespace]
vars = ["expected"]

[[job.check_namespace.step]]
name = "verify"
run = "echo namespace=$OJ_NAMESPACE"
"#;

/// A pair of projects sharing the same daemon (state directory).
struct ProjectPair {
    project_a: tempfile::TempDir,
    project_b: tempfile::TempDir,
    shared_state: tempfile::TempDir,
}

impl ProjectPair {
    fn new(name_a: &str, name_b: &str) -> Self {
        let project_a = tempfile::tempdir().unwrap();
        let project_b = tempfile::tempdir().unwrap();
        let shared_state = tempfile::tempdir().unwrap();

        // Initialize git in both
        for dir in [&project_a, &project_b] {
            std::process::Command::new("git")
                .args(["init"])
                .current_dir(dir.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("git init should work");
        }

        // Configure project names
        let config_a = format!("[project]\nname = \"{}\"\n", name_a);
        let config_b = format!("[project]\nname = \"{}\"\n", name_b);

        let config_dir_a = project_a.path().join(".oj");
        let config_dir_b = project_b.path().join(".oj");
        std::fs::create_dir_all(&config_dir_a).unwrap();
        std::fs::create_dir_all(&config_dir_b).unwrap();

        std::fs::write(config_dir_a.join("config.toml"), config_a).unwrap();
        std::fs::write(config_dir_b.join("config.toml"), config_b).unwrap();

        Self {
            project_a,
            project_b,
            shared_state,
        }
    }

    fn path_a(&self) -> &std::path::Path {
        self.project_a.path()
    }

    fn path_b(&self) -> &std::path::Path {
        self.project_b.path()
    }

    fn state_path(&self) -> &std::path::Path {
        self.shared_state.path()
    }

    /// Write a file to project A
    fn file_a(&self, path: impl AsRef<std::path::Path>, content: &str) {
        let full_path = self.path_a().join(path.as_ref());
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full_path, content).unwrap();
    }

    /// Write a file to project B
    fn file_b(&self, path: impl AsRef<std::path::Path>, content: &str) {
        let full_path = self.path_b().join(path.as_ref());
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full_path, content).unwrap();
    }

    /// Run oj command in project A's context (shares daemon with B)
    fn oj_a(&self) -> CliBuilder {
        cli()
            .pwd(self.path_a())
            .env("OJ_STATE_DIR", self.state_path())
            .env(
                "CLAUDE_CONFIG_DIR",
                PathBuf::from(self.state_path()).join("claude"),
            )
            .env("OJ_IDLE_GRACE_MS", "1000")
    }

    /// Run oj command in project B's context (shares daemon with A)
    fn oj_b(&self) -> CliBuilder {
        cli()
            .pwd(self.path_b())
            .env("OJ_STATE_DIR", self.state_path())
            .env(
                "CLAUDE_CONFIG_DIR",
                PathBuf::from(self.state_path()).join("claude"),
            )
            .env("OJ_IDLE_GRACE_MS", "1000")
    }

    /// Read the daemon log file contents (for debugging test failures)
    fn daemon_log(&self) -> String {
        let log_path = self.state_path().join("daemon.log");
        std::fs::read_to_string(&log_path).unwrap_or_else(|_| "(no daemon log)".to_string())
    }
}

impl Drop for ProjectPair {
    fn drop(&mut self) {
        // Stop daemon using either project context
        let mut cmd = self.oj_a().args(&["daemon", "stop", "--kill"]).command();
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let _ = cmd.status();
    }
}

// =============================================================================
// Test 1: Two projects with same job name don't interfere
// =============================================================================

#[test]
fn jobs_with_same_name_in_different_namespaces_dont_interfere() {
    let pair = ProjectPair::new("alpha", "beta");

    // Both projects have identical runbooks with the same job name
    pair.file_a(".oj/runbooks/work.toml", SIMPLE_JOB_RUNBOOK);
    pair.file_b(".oj/runbooks/work.toml", SIMPLE_JOB_RUNBOOK);

    // Start daemon (only needs to be done once since they share state)
    pair.oj_a().args(&["daemon", "start"]).passes();

    // Start workers in both projects - each is scoped to its namespace
    pair.oj_a().args(&["worker", "start", "runner"]).passes();
    pair.oj_b().args(&["worker", "start", "runner"]).passes();

    // Push items to each project's queue - same queue name, different namespaces
    pair.oj_a()
        .args(&["queue", "push", "tasks", r#"{"msg": "from-alpha"}"#])
        .passes();
    pair.oj_b()
        .args(&["queue", "push", "tasks", r#"{"msg": "from-beta"}"#])
        .passes();

    // Wait for jobs to complete in both namespaces
    let both_done = wait_for(SPEC_WAIT_MAX_MS * 2, || {
        let out_a = pair.oj_a().args(&["job", "list"]).passes().stdout();
        let out_b = pair.oj_b().args(&["job", "list"]).passes().stdout();
        out_a.contains("completed") && out_b.contains("completed")
    });

    if !both_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", pair.daemon_log());
    }
    assert!(both_done, "jobs should complete in both namespaces");

    // Verify each project only sees its own job (namespace scoping)
    let jobs_a = pair.oj_a().args(&["job", "list"]).passes().stdout();
    let jobs_b = pair.oj_b().args(&["job", "list"]).passes().stdout();

    // Jobs should be scoped - project A shouldn't see project B's jobs and vice versa
    // Count how many completed jobs each project sees
    let completed_count_a = jobs_a.matches("completed").count();
    let completed_count_b = jobs_b.matches("completed").count();

    assert_eq!(
        completed_count_a, 1,
        "project A should see exactly 1 completed job, got:\n{}",
        jobs_a
    );
    assert_eq!(
        completed_count_b, 1,
        "project B should see exactly 1 completed job, got:\n{}",
        jobs_b
    );
}

// =============================================================================
// Test 2: Queue items are scoped by namespace
// =============================================================================

#[test]
fn queue_items_are_scoped_by_namespace() {
    let pair = ProjectPair::new("proj-one", "proj-two");

    // Use a runbook with only a queue (no worker) so items stay pending
    const QUEUE_ONLY_RUNBOOK: &str = r#"
[queue.tasks]
type = "persisted"
vars = ["msg"]
"#;

    pair.file_a(".oj/runbooks/work.toml", QUEUE_ONLY_RUNBOOK);
    pair.file_b(".oj/runbooks/work.toml", QUEUE_ONLY_RUNBOOK);

    // Start daemon
    pair.oj_a().args(&["daemon", "start"]).passes();

    // Push items to each project's queue
    pair.oj_a()
        .args(&["queue", "push", "tasks", r#"{"msg": "item-one"}"#])
        .passes();
    pair.oj_a()
        .args(&["queue", "push", "tasks", r#"{"msg": "item-two"}"#])
        .passes();
    pair.oj_b()
        .args(&["queue", "push", "tasks", r#"{"msg": "item-beta"}"#])
        .passes();

    // Verify queue scoping - each project sees only its own items
    let queue_a = pair
        .oj_a()
        .args(&["queue", "show", "tasks"])
        .passes()
        .stdout();
    let queue_b = pair
        .oj_b()
        .args(&["queue", "show", "tasks"])
        .passes()
        .stdout();

    // Verify the actual item data to ensure no cross-contamination
    // Project A should see its items but not project B's
    assert!(
        queue_a.contains("item-one") && queue_a.contains("item-two"),
        "project A queue should contain its items, got:\n{}",
        queue_a
    );
    assert!(
        !queue_a.contains("item-beta"),
        "project A queue should NOT contain project B's items, got:\n{}",
        queue_a
    );

    // Project B should see only its item
    assert!(
        queue_b.contains("item-beta"),
        "project B queue should contain its items, got:\n{}",
        queue_b
    );
    assert!(
        !queue_b.contains("item-one") && !queue_b.contains("item-two"),
        "project B queue should NOT contain project A's items, got:\n{}",
        queue_b
    );
}

// =============================================================================
// Test 3: OJ_NAMESPACE propagates through shell steps
// =============================================================================

#[test]
fn oj_namespace_propagates_to_shell_steps() {
    let pair = ProjectPair::new("namespace-test", "other-ns");

    pair.file_a(".oj/runbooks/check.toml", NAMESPACE_ECHO_RUNBOOK);

    pair.oj_a().args(&["daemon", "start"]).passes();

    // Run a command that triggers a job echoing $OJ_NAMESPACE
    pair.oj_a()
        .args(&["run", "check", "namespace-test"])
        .passes();

    // Wait for job to complete
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = pair.oj_a().args(&["job", "list"]).passes().stdout();
        out.contains("completed")
    });

    if !completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", pair.daemon_log());
    }
    assert!(completed, "job should complete");

    // Get the job ID to check its output
    let jobs_out = pair.oj_a().args(&["job", "list"]).passes().stdout();
    let job_id = jobs_out
        .lines()
        .find(|l| l.contains("check_namespace"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find job ID");

    // Check job logs for the namespace echoed by the shell step
    let job_logs = pair.oj_a().args(&["logs", job_id]).passes().stdout();

    assert!(
        job_logs.contains("namespace=namespace-test"),
        "shell step should see OJ_NAMESPACE=namespace-test, got:\n{}",
        job_logs
    );
}

// =============================================================================
// Test 4: Workers are namespace-scoped (same name in different namespaces)
// =============================================================================

#[test]
fn workers_in_different_namespaces_are_independent() {
    let pair = ProjectPair::new("worker-ns-a", "worker-ns-b");

    // Both projects have identical runbooks
    pair.file_a(".oj/runbooks/work.toml", SIMPLE_JOB_RUNBOOK);
    pair.file_b(".oj/runbooks/work.toml", SIMPLE_JOB_RUNBOOK);

    pair.oj_a().args(&["daemon", "start"]).passes();

    // Start workers in both namespaces (same worker name, different namespace)
    pair.oj_a().args(&["worker", "start", "runner"]).passes();
    pair.oj_b().args(&["worker", "start", "runner"]).passes();

    // Push items to each queue
    pair.oj_a()
        .args(&["queue", "push", "tasks", r#"{"msg": "alpha-work"}"#])
        .passes();
    pair.oj_b()
        .args(&["queue", "push", "tasks", r#"{"msg": "beta-work"}"#])
        .passes();

    // Wait for both jobs to complete
    let both_done = wait_for(SPEC_WAIT_MAX_MS * 2, || {
        let out_a = pair.oj_a().args(&["job", "list"]).passes().stdout();
        let out_b = pair.oj_b().args(&["job", "list"]).passes().stdout();
        out_a.contains("completed") && out_b.contains("completed")
    });

    if !both_done {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", pair.daemon_log());
    }
    assert!(
        both_done,
        "jobs should complete via namespace-scoped workers"
    );

    // Verify worker list shows both workers with their respective namespaces
    // The daemon shows all workers (not filtered by namespace) but with PROJECT column
    let workers = pair.oj_a().args(&["worker", "list"]).passes().stdout();
    assert!(
        workers.contains("worker-ns-a") && workers.contains("worker-ns-b"),
        "worker list should show both namespaces, got:\n{}",
        workers
    );
}

// =============================================================================
// Test 5: Namespace derived from config.toml takes precedence over directory name
// =============================================================================

#[test]
fn config_name_takes_precedence_over_directory_name() {
    // Create a project with a config name that differs from directory basename
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/config.toml",
        "[project]\nname = \"custom-namespace\"\n",
    );
    temp.file(".oj/runbooks/check.toml", NAMESPACE_ECHO_RUNBOOK);

    temp.oj().args(&["daemon", "start"]).passes();

    // Run command that triggers a job echoing namespace
    temp.oj()
        .args(&["run", "check", "custom-namespace"])
        .passes();

    // Wait for job to complete
    let completed = wait_for(SPEC_WAIT_MAX_MS, || {
        let out = temp.oj().args(&["job", "list"]).passes().stdout();
        out.contains("completed")
    });

    if !completed {
        eprintln!("=== DAEMON LOG ===\n{}\n=== END LOG ===", temp.daemon_log());
    }
    assert!(completed, "job should complete");

    // Get job ID
    let jobs_out = temp.oj().args(&["job", "list"]).passes().stdout();
    let job_id = jobs_out
        .lines()
        .find(|l| l.contains("check_namespace"))
        .and_then(|l| l.split_whitespace().next())
        .expect("should find job ID");

    // Verify the namespace is from config, not from the temp directory name
    // Check via job logs which shows the echo output
    let job_logs = temp.oj().args(&["logs", job_id]).passes().stdout();

    assert!(
        job_logs.contains("namespace=custom-namespace"),
        "namespace should be 'custom-namespace' from config.toml, not directory basename, got:\n{}",
        job_logs
    );
}
