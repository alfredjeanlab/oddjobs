// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::time::Instant;

use tempfile::tempdir;

use oj_core::{StepOutcome, StepStatus};

use super::{
    empty_orphans, empty_state, handle_query, make_cron, make_job, make_worker, Query, Response,
};

#[test]
fn list_projects_empty_state() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    let response = handle_query(Query::ListProjects, &state, &empty_orphans(), temp.path(), start);
    match response {
        Response::Projects { projects } => {
            assert!(projects.is_empty());
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_from_workers() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        let mut worker = make_worker("build-worker", "myapp", "build", 1);
        worker.project_path = std::path::PathBuf::from("/home/user/myapp");
        s.workers.insert("myapp/build-worker".to_string(), worker);
    }

    let response = handle_query(Query::ListProjects, &state, &empty_orphans(), temp.path(), start);
    match response {
        Response::Projects { projects } => {
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].name, "myapp");
            assert_eq!(projects[0].root, std::path::PathBuf::from("/home/user/myapp"));
            assert_eq!(projects[0].workers, 1);
            assert_eq!(projects[0].active_jobs, 0);
            assert_eq!(projects[0].crons, 0);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_from_crons() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.crons.insert(
            "webapp/health".to_string(),
            make_cron("health", "webapp", "/home/user/webapp"),
        );
    }

    let response = handle_query(Query::ListProjects, &state, &empty_orphans(), temp.path(), start);
    match response {
        Response::Projects { projects } => {
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].name, "webapp");
            assert_eq!(projects[0].root, std::path::PathBuf::from("/home/user/webapp"));
            assert_eq!(projects[0].crons, 1);
            assert_eq!(projects[0].workers, 0);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_from_jobs_with_agents() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.jobs.insert(
            "p1".to_string(),
            make_job(
                "p1",
                "fix/bug",
                "oddjobs",
                "work",
                StepStatus::Running,
                StepOutcome::Running,
                Some("agent-1"),
                1000,
            ),
        );
        // Add a stopped worker so project_path can be resolved
        let mut worker = make_worker("w1", "oddjobs", "q1", 0);
        worker.status = "stopped".to_string();
        worker.project_path = std::path::PathBuf::from("/home/user/oddjobs");
        s.workers.insert("oddjobs/w1".to_string(), worker);
    }

    let response = handle_query(Query::ListProjects, &state, &empty_orphans(), temp.path(), start);
    match response {
        Response::Projects { projects } => {
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].name, "oddjobs");
            assert_eq!(projects[0].active_jobs, 1);
            assert_eq!(projects[0].active_agents, 1);
            assert_eq!(projects[0].root, std::path::PathBuf::from("/home/user/oddjobs"));
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_multiple_namespaces_sorted() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        let mut w1 = make_worker("w1", "zebra", "q1", 1);
        w1.project_path = std::path::PathBuf::from("/home/user/zebra");
        s.workers.insert("zebra/w1".to_string(), w1);

        let mut w2 = make_worker("w2", "alpha", "q2", 0);
        w2.project_path = std::path::PathBuf::from("/home/user/alpha");
        s.workers.insert("alpha/w2".to_string(), w2);
    }

    let response = handle_query(Query::ListProjects, &state, &empty_orphans(), temp.path(), start);
    match response {
        Response::Projects { projects } => {
            assert_eq!(projects.len(), 2);
            assert_eq!(projects[0].name, "alpha");
            assert_eq!(projects[1].name, "zebra");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_excludes_stopped_only() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        // Stopped worker with no active jobs or crons
        let mut w = make_worker("w1", "inactive", "q1", 0);
        w.status = "stopped".to_string();
        w.project_path = std::path::PathBuf::from("/home/user/inactive");
        s.workers.insert("inactive/w1".to_string(), w);
    }

    let response = handle_query(Query::ListProjects, &state, &empty_orphans(), temp.path(), start);
    match response {
        Response::Projects { projects } => {
            assert!(projects.is_empty(), "stopped-only projects should be excluded");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[test]
fn list_projects_excludes_terminal_jobs() {
    let state = empty_state();
    let temp = tempdir().unwrap();
    let start = Instant::now();

    {
        let mut s = state.lock();
        s.jobs.insert(
            "p1".to_string(),
            make_job(
                "p1",
                "fix/done",
                "oddjobs",
                "done",
                StepStatus::Completed,
                StepOutcome::Completed,
                None,
                1000,
            ),
        );
    }

    let response = handle_query(Query::ListProjects, &state, &empty_orphans(), temp.path(), start);
    match response {
        Response::Projects { projects } => {
            assert!(projects.is_empty(), "terminal jobs should not create active projects");
        }
        other => panic!("unexpected response: {:?}", other),
    }
}
