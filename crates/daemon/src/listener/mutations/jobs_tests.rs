// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;

use tempfile::tempdir;

use oj_core::StepStatus;

use crate::protocol::Response;

use super::super::PruneFlags;
use super::{handle_job_cancel, handle_job_prune, handle_job_resume, handle_job_resume_all};
use crate::listener::test_ctx;
use crate::listener::test_fixtures::{
    load_runbook_into_state, load_runbook_json_into_state, make_agent_runbook_json,
    make_breadcrumb, make_job, make_job_ns, make_shell_runbook_json,
};

// --- handle_job_resume tests ---

#[test]
fn resume_existing_job_emits_event() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        s.jobs.insert("job-1".to_string(), make_job("job-1", "work"));
    }

    let result = handle_job_resume(
        &ctx,
        "job-1".to_string(),
        Some("try again".to_string()),
        HashMap::new(),
        false,
    );

    assert!(matches!(result, Ok(Response::Ok)));
}

#[test]
fn resume_nonexistent_job_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let result = handle_job_resume(&ctx, "nonexistent".to_string(), None, HashMap::new(), false);

    match result {
        Ok(Response::Error { message }) => {
            assert!(
                message.contains("not found"),
                "expected 'not found' in message, got: {}",
                message
            );
        }
        other => panic!("expected Response::Error, got: {:?}", other),
    }
}

#[test]
fn resume_orphan_without_runbook_hash_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let mut bc = make_breadcrumb("orphan-1");
    bc.runbook_hash = String::new();
    *ctx.orphans.lock() = vec![bc];

    let result = handle_job_resume(&ctx, "orphan-1".to_string(), None, HashMap::new(), false);

    match result {
        Ok(Response::Error { message }) => {
            assert!(
                message.contains("orphaned") && message.contains("breadcrumb missing"),
                "unexpected error: {}",
                message
            );
        }
        other => panic!("expected Response::Error, got: {:?}", other),
    }
}

#[test]
fn resume_orphan_without_runbook_in_state_returns_error() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    *ctx.orphans.lock() = vec![make_breadcrumb("orphan-2")];

    let result = handle_job_resume(&ctx, "orphan-2".to_string(), None, HashMap::new(), false);

    match result {
        Ok(Response::Error { message }) => {
            assert!(
                message.contains("orphaned") && message.contains("runbook is no longer"),
                "unexpected error: {}",
                message
            );
        }
        other => panic!("expected Response::Error, got: {:?}", other),
    }
}

#[test]
fn resume_orphan_with_runbook_reconstructs_and_resumes() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    load_runbook_into_state(&ctx.state, "hash456");
    *ctx.orphans.lock() = vec![make_breadcrumb("orphan-3")];

    let result = handle_job_resume(
        &ctx,
        "orphan-3".to_string(),
        Some("fix it".to_string()),
        HashMap::new(),
        false,
    );

    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
    assert!(ctx.orphans.lock().is_empty(), "orphan should be removed");
}

#[test]
fn resume_orphan_by_prefix() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    load_runbook_into_state(&ctx.state, "hash456");
    *ctx.orphans.lock() = vec![make_breadcrumb("orphan-long-uuid-string-12345")];

    let result = handle_job_resume(
        &ctx,
        "orphan-long".to_string(),
        Some("try again".to_string()),
        HashMap::new(),
        false,
    );

    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
    assert!(ctx.orphans.lock().is_empty());
}

// --- resume with runbook tests (parameterized) ---

#[yare::parameterized(
    agent_step_no_message = { true, "work", "work", None },
    agent_step_with_message = { true, "work", "work", Some("I fixed the issue") },
    shell_step_no_message = { false, "build", "build", None },
    failed_job_no_message = { true, "work", "failed", None },
)]
fn resume_with_runbook(is_agent: bool, step_name: &str, job_step: &str, message: Option<&str>) {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let runbook_hash = "test-runbook-hash";
    let runbook_json = if is_agent {
        make_agent_runbook_json("test", step_name)
    } else {
        make_shell_runbook_json("test", step_name)
    };
    load_runbook_json_into_state(&ctx.state, runbook_hash, runbook_json);

    let job_id = format!("job-{}", job_step);
    let mut job = make_job(&job_id, job_step);
    job.runbook_hash = runbook_hash.to_string();
    ctx.state.lock().jobs.insert(job_id.clone(), job);

    let result =
        handle_job_resume(&ctx, job_id, message.map(|s| s.to_string()), HashMap::new(), false);
    assert!(matches!(result, Ok(Response::Ok)), "got: {:?}", result);
}

// --- handle_job_cancel tests (parameterized) ---

#[yare::parameterized(
    single_running = {
        &[("job-1", "work")],
        &["job-1"],
        &["job-1"], &[], &[]
    },
    nonexistent = {
        &[],
        &["no-such-pipe"],
        &[], &[], &["no-such-pipe"]
    },
    already_terminal = {
        &[("job-done", "done"), ("job-failed", "failed"), ("job-cancelled", "cancelled")],
        &["job-done", "job-failed", "job-cancelled"],
        &[], &["job-done", "job-failed", "job-cancelled"], &[]
    },
    mixed = {
        &[("job-a", "build"), ("job-b", "test"), ("job-c", "done")],
        &["job-a", "job-b", "job-c", "job-d"],
        &["job-a", "job-b"], &["job-c"], &["job-d"]
    },
    empty_ids = {
        &[], &[], &[], &[], &[]
    },
)]
fn cancel_job(
    jobs: &[(&str, &str)],
    cancel_ids: &[&str],
    exp_cancelled: &[&str],
    exp_terminal: &[&str],
    exp_not_found: &[&str],
) {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        for &(id, step) in jobs {
            s.jobs.insert(id.to_string(), make_job(id, step));
        }
    }

    let ids = cancel_ids.iter().map(|s| s.to_string()).collect();
    let result = handle_job_cancel(&ctx, ids);

    match result {
        Ok(Response::JobsCancelled { cancelled, already_terminal, not_found }) => {
            assert_eq!(cancelled, exp_cancelled);
            assert_eq!(already_terminal, exp_terminal);
            assert_eq!(not_found, exp_not_found);
        }
        other => panic!("expected JobsCancelled, got: {:?}", other),
    }
}

// --- handle_job_prune tests ---

#[test]
fn job_prune_all_without_namespace_prunes_across_all_projects() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(&ctx.logs_path).unwrap();

    {
        let mut s = ctx.state.lock();
        s.jobs.insert("job-a".to_string(), make_job_ns("job-a", "done", "proj-alpha"));
        s.jobs.insert("job-b".to_string(), make_job_ns("job-b", "failed", "proj-beta"));
        s.jobs.insert("job-c".to_string(), make_job_ns("job-c", "cancelled", "proj-gamma"));
        // Non-terminal job should be skipped
        s.jobs.insert("job-d".to_string(), make_job_ns("job-d", "work", "proj-alpha"));
    }

    let flags = PruneFlags { all: true, dry_run: false, project: None };
    let result = handle_job_prune(&ctx, &flags, false, false);

    match result {
        Ok(Response::JobsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 3);
            let pruned_ids: Vec<&str> = pruned.iter().map(|e| e.id.as_str()).collect();
            assert!(pruned_ids.contains(&"job-a"));
            assert!(pruned_ids.contains(&"job-b"));
            assert!(pruned_ids.contains(&"job-c"));
            assert_eq!(skipped, 1);
        }
        other => panic!("expected JobsPruned, got: {:?}", other),
    }
}

#[test]
fn job_prune_all_with_namespace_only_prunes_matching_project() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(&ctx.logs_path).unwrap();

    {
        let mut s = ctx.state.lock();
        s.jobs.insert("job-a".to_string(), make_job_ns("job-a", "done", "proj-alpha"));
        s.jobs.insert("job-b".to_string(), make_job_ns("job-b", "failed", "proj-beta"));
        s.jobs.insert("job-c".to_string(), make_job_ns("job-c", "cancelled", "proj-alpha"));
    }

    let flags = PruneFlags { all: true, dry_run: false, project: Some("proj-alpha") };
    let result = handle_job_prune(&ctx, &flags, false, false);

    match result {
        Ok(Response::JobsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 2);
            let pruned_ids: Vec<&str> = pruned.iter().map(|e| e.id.as_str()).collect();
            assert!(pruned_ids.contains(&"job-a"));
            assert!(pruned_ids.contains(&"job-c"));
            assert_eq!(skipped, 0);
        }
        other => panic!("expected JobsPruned, got: {:?}", other),
    }
}

#[test]
fn job_prune_skips_non_terminal_steps() {
    let dir = tempdir().unwrap();
    let mut ctx = test_ctx(dir.path());
    ctx.logs_path = dir.path().join("logs");
    std::fs::create_dir_all(&ctx.logs_path).unwrap();

    {
        let mut s = ctx.state.lock();
        s.jobs.insert("job-running".to_string(), make_job("job-running", "implement"));
        s.jobs.insert("job-work".to_string(), make_job("job-work", "work"));
    }

    let flags = PruneFlags { all: true, dry_run: false, project: None };
    let result = handle_job_prune(&ctx, &flags, false, false);

    match result {
        Ok(Response::JobsPruned { pruned, skipped }) => {
            assert_eq!(pruned.len(), 0);
            assert_eq!(skipped, 2);
        }
        other => panic!("expected JobsPruned, got: {:?}", other),
    }
}

// --- handle_job_resume_all tests ---

#[test]
fn resume_all_resumes_waiting_jobs() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        let mut job = make_job("job-1", "work");
        job.step_status = StepStatus::Waiting(None);
        s.jobs.insert("job-1".to_string(), job);
    }

    let result = handle_job_resume_all(&ctx, false);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert_eq!(resumed, vec!["job-1"]);
            assert!(skipped.is_empty());
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}

#[test]
fn resume_all_resumes_failed_jobs() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        let mut job = make_job("job-1", "work");
        job.step_status = StepStatus::Failed;
        s.jobs.insert("job-1".to_string(), job);
    }

    let result = handle_job_resume_all(&ctx, false);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert_eq!(resumed, vec!["job-1"]);
            assert!(skipped.is_empty());
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}

#[test]
fn resume_all_skips_running_jobs_without_kill() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        let job = make_job("job-1", "work");
        s.jobs.insert("job-1".to_string(), job);
    }

    let result = handle_job_resume_all(&ctx, false);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert!(resumed.is_empty());
            assert_eq!(skipped.len(), 1);
            assert_eq!(skipped[0].0, "job-1");
            assert!(skipped[0].1.contains("--kill"));
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}

#[test]
fn resume_all_with_kill_resumes_running_jobs() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        let job = make_job("job-1", "work");
        s.jobs.insert("job-1".to_string(), job);
    }

    let result = handle_job_resume_all(&ctx, true);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert_eq!(resumed, vec!["job-1"]);
            assert!(skipped.is_empty());
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}

#[test]
fn resume_all_skips_terminal_jobs() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    {
        let mut s = ctx.state.lock();
        let done_job = make_job("job-done", "done");
        s.jobs.insert("job-done".to_string(), done_job);

        let mut waiting_job = make_job("job-wait", "work");
        waiting_job.step_status = StepStatus::Waiting(None);
        s.jobs.insert("job-wait".to_string(), waiting_job);
    }

    let result = handle_job_resume_all(&ctx, false);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert_eq!(resumed, vec!["job-wait"]);
            assert!(skipped.is_empty());
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}

#[test]
fn resume_all_returns_empty_when_no_jobs() {
    let dir = tempdir().unwrap();
    let ctx = test_ctx(dir.path());

    let result = handle_job_resume_all(&ctx, false);
    match result {
        Ok(Response::JobsResumed { resumed, skipped }) => {
            assert!(resumed.is_empty());
            assert!(skipped.is_empty());
        }
        other => panic!("expected JobsResumed, got: {:?}", other),
    }
}
