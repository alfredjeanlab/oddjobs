// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use std::fs;
use tempfile::TempDir;

fn write_hcl(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

const CMD_RUNBOOK: &str = r#"
command "deploy" {
  args = "<env>"
  run  = "echo deploy"
}
"#;

const CMD_RUNBOOK_B: &str = r#"
command "build" {
  args = "<name> <instructions>"
  run  = "echo build"
}

command "test" {
  run  = "echo test"
}
"#;

const WORKER_RUNBOOK: &str = r#"
queue "jobs" {
  type = "external"
  list = "echo []"
  take = "echo ok"
}

worker "builder" {
  source  = { queue = "jobs" }
  handler = { pipeline = "build" }
}

pipeline "build" {
  step "run" {
    run = "echo build"
  }
}
"#;

const QUEUE_RUNBOOK: &str = r#"
queue "tasks" {
  type = "persisted"
  vars = ["title"]
}
"#;

#[test]
fn find_command_top_level() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "deploy.hcl", CMD_RUNBOOK);

    let result = find_runbook_by_command(tmp.path(), "deploy").unwrap();
    assert!(result.is_some());
    assert!(result.unwrap().get_command("deploy").is_some());
}

#[test]
fn find_command_in_subdirectory() {
    let tmp = TempDir::new().unwrap();
    let sub = tmp.path().join("ops");
    fs::create_dir(&sub).unwrap();
    write_hcl(&sub, "deploy.hcl", CMD_RUNBOOK);

    let result = find_runbook_by_command(tmp.path(), "deploy").unwrap();
    assert!(result.is_some());
    assert!(result.unwrap().get_command("deploy").is_some());
}

#[test]
fn find_worker_in_subdirectory() {
    let tmp = TempDir::new().unwrap();
    let sub = tmp.path().join("ci");
    fs::create_dir(&sub).unwrap();
    write_hcl(&sub, "build.hcl", WORKER_RUNBOOK);

    let result = find_runbook_by_worker(tmp.path(), "builder").unwrap();
    assert!(result.is_some());
    assert!(result.unwrap().get_worker("builder").is_some());
}

#[test]
fn find_queue_in_subdirectory() {
    let tmp = TempDir::new().unwrap();
    let sub = tmp.path().join("queues");
    fs::create_dir(&sub).unwrap();
    write_hcl(&sub, "tasks.hcl", QUEUE_RUNBOOK);

    let result = find_runbook_by_queue(tmp.path(), "tasks").unwrap();
    assert!(result.is_some());
    assert!(result.unwrap().get_queue("tasks").is_some());
}

#[test]
fn find_in_nested_subdirectory() {
    let tmp = TempDir::new().unwrap();
    let nested = tmp.path().join("a").join("b");
    fs::create_dir_all(&nested).unwrap();
    write_hcl(&nested, "deploy.hcl", CMD_RUNBOOK);

    let result = find_runbook_by_command(tmp.path(), "deploy").unwrap();
    assert!(result.is_some());
}

#[test]
fn returns_none_for_missing_name() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "deploy.hcl", CMD_RUNBOOK);

    let result = find_runbook_by_command(tmp.path(), "nonexistent").unwrap();
    assert!(result.is_none());
}

#[test]
fn returns_none_for_missing_directory() {
    let result = find_runbook_by_command(Path::new("/nonexistent"), "deploy").unwrap();
    assert!(result.is_none());
}

#[test]
fn duplicate_across_files_is_error() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "a.hcl", CMD_RUNBOOK);
    let sub = tmp.path().join("sub");
    fs::create_dir(&sub).unwrap();
    write_hcl(&sub, "b.hcl", CMD_RUNBOOK);

    let result = find_runbook_by_command(tmp.path(), "deploy");
    assert!(matches!(result, Err(FindError::Duplicate(_))));
}

#[test]
fn invalid_runbook_is_skipped() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "bad.hcl", "this is not valid HCL {{{}}}");
    write_hcl(tmp.path(), "deploy.hcl", CMD_RUNBOOK);

    let result = find_runbook_by_command(tmp.path(), "deploy").unwrap();
    assert!(result.is_some());
    assert!(result.unwrap().get_command("deploy").is_some());
}

#[test]
fn only_invalid_runbooks_returns_not_found_skipped() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "bad.hcl", "this is not valid HCL {{{}}}");

    let err = find_runbook_by_command(tmp.path(), "deploy").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("skipped due to errors"),
        "expected skipped mention, got: {msg}"
    );
    assert!(msg.contains("bad.hcl"), "expected file path, got: {msg}");
}

#[test]
fn multiple_invalid_runbooks_lists_all_paths() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "bad1.hcl", "not valid");
    write_hcl(tmp.path(), "bad2.hcl", "also not valid");

    let err = find_runbook_by_command(tmp.path(), "deploy").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("2 runbook(s) skipped"), "got: {msg}");
    assert!(msg.contains("bad1.hcl"), "got: {msg}");
    assert!(msg.contains("bad2.hcl"), "got: {msg}");
}

#[test]
fn collect_all_commands_multiple_files() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "deploy.hcl", CMD_RUNBOOK);
    write_hcl(tmp.path(), "build.hcl", CMD_RUNBOOK_B);

    let commands = collect_all_commands(tmp.path()).unwrap();
    let names: Vec<&str> = commands.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["build", "deploy", "test"]);
}

#[test]
fn collect_all_commands_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let commands = collect_all_commands(tmp.path()).unwrap();
    assert!(commands.is_empty());
}

#[test]
fn collect_all_commands_missing_dir() {
    let commands = collect_all_commands(Path::new("/nonexistent")).unwrap();
    assert!(commands.is_empty());
}

#[test]
fn collect_all_commands_skips_invalid() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "bad.hcl", "this is not valid HCL {{{}}}");
    write_hcl(tmp.path(), "deploy.hcl", CMD_RUNBOOK);

    let commands = collect_all_commands(tmp.path()).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "deploy");
}
