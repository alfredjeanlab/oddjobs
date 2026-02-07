// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use std::fs;
use tempfile::TempDir;

fn write_hcl(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

// ============================================================================
// Cross-Runbook Duplicate Detection (validate_runbook_dir)
// ============================================================================

#[test]
fn validate_duplicate_command_across_files() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "a.hcl", CMD_RUNBOOK);
    write_hcl(tmp.path(), "b.hcl", CMD_RUNBOOK); // same "deploy" command

    let errs = validate_runbook_dir(tmp.path()).unwrap_err();
    assert!(!errs.is_empty());
    let msg = errs[0].to_string();
    assert!(
        msg.contains("command") && msg.contains("deploy"),
        "expected duplicate command error, got: {msg}"
    );
}

#[test]
fn validate_duplicate_agent_across_files() {
    let tmp = TempDir::new().unwrap();
    let agent_hcl = r#"
agent "planner" {
  run = "claude"
}
"#;
    write_hcl(tmp.path(), "a.hcl", agent_hcl);
    write_hcl(tmp.path(), "b.hcl", agent_hcl);

    let errs = validate_runbook_dir(tmp.path()).unwrap_err();
    assert!(!errs.is_empty());
    let msg = errs[0].to_string();
    assert!(
        msg.contains("agent") && msg.contains("planner"),
        "expected duplicate agent error, got: {msg}"
    );
}

#[test]
fn validate_duplicate_job_across_files() {
    let tmp = TempDir::new().unwrap();
    let job_hcl = r#"
job "build" {
  step "run" {
    run = "echo build"
  }
}
"#;
    write_hcl(tmp.path(), "a.hcl", job_hcl);
    write_hcl(tmp.path(), "b.hcl", job_hcl);

    let errs = validate_runbook_dir(tmp.path()).unwrap_err();
    assert!(!errs.is_empty());
    let msg = errs[0].to_string();
    assert!(
        msg.contains("job") && msg.contains("build"),
        "expected duplicate job error, got: {msg}"
    );
}

#[test]
fn validate_same_name_different_entity_types_is_ok() {
    let tmp = TempDir::new().unwrap();
    // "build" used as both a command and a job - different types, should be fine
    write_hcl(
        tmp.path(),
        "a.hcl",
        r#"
command "build" {
  run = { job = "build" }
}

job "build" {
  step "run" {
    run = "echo build"
  }
}
"#,
    );

    assert!(validate_runbook_dir(tmp.path()).is_ok());
}

#[test]
fn validate_no_duplicates_across_files() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "deploy.hcl", CMD_RUNBOOK);
    write_hcl(tmp.path(), "build.hcl", CMD_RUNBOOK_B);

    assert!(validate_runbook_dir(tmp.path()).is_ok());
}

#[test]
fn validate_missing_dir_is_ok() {
    assert!(validate_runbook_dir(Path::new("/nonexistent")).is_ok());
}

#[test]
fn validate_skips_invalid_runbooks() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "bad.hcl", "this is not valid HCL {{{}}}");
    write_hcl(tmp.path(), "deploy.hcl", CMD_RUNBOOK);

    // Should succeed since the bad file is skipped
    assert!(validate_runbook_dir(tmp.path()).is_ok());
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
queue "tasks" {
  type = "external"
  list = "echo []"
  take = "echo ok"
}

worker "builder" {
  source  = { queue = "tasks" }
  handler = { job = "build" }
}

job "build" {
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
    let names: Vec<&str> = commands.iter().map(|(n, _, _)| n.as_str()).collect();
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

#[test]
fn collect_all_queues_multiple_files() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "worker.hcl", WORKER_RUNBOOK);
    write_hcl(tmp.path(), "tasks.hcl", QUEUE_RUNBOOK);

    let queues = collect_all_queues(tmp.path()).unwrap();
    let names: Vec<&str> = queues.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["tasks", "tasks"]);
}

#[test]
fn collect_all_queues_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let queues = collect_all_queues(tmp.path()).unwrap();
    assert!(queues.is_empty());
}

#[test]
fn collect_all_queues_missing_dir() {
    let queues = collect_all_queues(Path::new("/nonexistent")).unwrap();
    assert!(queues.is_empty());
}

#[test]
fn collect_all_queues_skips_invalid() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "bad.hcl", "this is not valid HCL {{{}}}");
    write_hcl(tmp.path(), "tasks.hcl", QUEUE_RUNBOOK);

    let queues = collect_all_queues(tmp.path()).unwrap();
    assert_eq!(queues.len(), 1);
    assert_eq!(queues[0].0, "tasks");
}

// extract_file_comment tests

#[test]
fn extract_comment_multi_paragraph() {
    let content = "# Build Runbook\n# Feature development workflow\n#\n# Usage:\n#   oj run build <name>\n\ncommand \"build\" {}\n";
    let comment = extract_file_comment(content).unwrap();
    assert_eq!(comment.short, "Build Runbook\nFeature development workflow");
    assert_eq!(comment.long, "Usage:\n  oj run build <name>");
}

#[test]
fn extract_comment_single_line() {
    let content = "# Simple command\n\ncommand \"test\" {}\n";
    let comment = extract_file_comment(content).unwrap();
    assert_eq!(comment.short, "Simple command");
    assert!(comment.long.is_empty());
}

#[test]
fn extract_comment_no_comment() {
    let content = "command \"test\" {}\n";
    assert!(extract_file_comment(content).is_none());
}

#[test]
fn extract_comment_leading_blank_lines() {
    let content = "\n\n# After blanks\n\ncommand \"test\" {}\n";
    let comment = extract_file_comment(content).unwrap();
    assert_eq!(comment.short, "After blanks");
}

#[test]
fn extract_comment_bare_hash() {
    let content = "#\n# Title\n#\n# Body\n\ncommand \"test\" {}\n";
    let comment = extract_file_comment(content).unwrap();
    // First bare # produces empty string, which is the split point
    // So short = "" (before the split), long = "Title\n\nBody" (wait, no)
    // Actually: lines = ["", "Title", "", "Body"]
    // split_pos = 0 (first empty), short = [] (empty), long = ["Title", "", "Body"]
    assert_eq!(comment.short, "");
    assert_eq!(comment.long, "Title\n\nBody");
}

#[test]
fn find_command_with_comment_returns_data() {
    let tmp = TempDir::new().unwrap();
    let content = "# Build Runbook\n# Feature workflow\n#\n# Usage:\n#   oj run build <name>\n\ncommand \"build\" {\n  args = \"<name>\"\n  run  = \"echo build\"\n}\n";
    write_hcl(tmp.path(), "build.hcl", content);

    let result = find_command_with_comment(tmp.path(), "build").unwrap();
    assert!(result.is_some());
    let (cmd, comment) = result.unwrap();
    assert_eq!(cmd.name, "build");
    let comment = comment.unwrap();
    assert_eq!(comment.short, "Build Runbook\nFeature workflow");
    assert!(comment.long.contains("Usage:"));
}

#[test]
fn find_command_with_comment_missing() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "deploy.hcl", CMD_RUNBOOK);

    let result = find_command_with_comment(tmp.path(), "nonexistent").unwrap();
    assert!(result.is_none());
}

#[test]
fn collect_all_commands_populates_description_from_comment() {
    let tmp = TempDir::new().unwrap();
    let content = "# Build Runbook\n# Feature workflow: init → plan → implement\n\ncommand \"build\" {\n  args = \"<name>\"\n  run  = \"echo build\"\n}\n";
    write_hcl(tmp.path(), "build.hcl", content);

    let commands = collect_all_commands(tmp.path()).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].2.as_deref(),
        Some("Feature workflow: init → plan → implement")
    );
}

#[test]
fn collect_all_commands_single_line_comment_used_as_description() {
    let tmp = TempDir::new().unwrap();
    let content = "# Simple command\n\ncommand \"test\" {\n  run = \"echo test\"\n}\n";
    write_hcl(tmp.path(), "test.hcl", content);

    let commands = collect_all_commands(tmp.path()).unwrap();
    assert_eq!(commands[0].2.as_deref(), Some("Simple command"));
}

// ============================================================================
// collect_all_workers / collect_all_crons tests
// ============================================================================

const CRON_RUNBOOK: &str = r#"
cron "daily-backup" {
  interval = "24h"
  run      = { job = "backup" }
}

job "backup" {
  step "run" {
    run = "echo backup"
  }
}
"#;

const WORKER_RUNBOOK_B: &str = r#"
queue "issues" {
  type = "external"
  list = "echo []"
  take = "echo ok"
}

worker "triager" {
  source  = { queue = "issues" }
  handler = { job = "triage" }
}

job "triage" {
  step "run" {
    run = "echo triage"
  }
}
"#;

#[test]
fn collect_all_workers_multiple_files() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "build.hcl", WORKER_RUNBOOK);
    write_hcl(tmp.path(), "triage.hcl", WORKER_RUNBOOK_B);

    let workers = collect_all_workers(tmp.path()).unwrap();
    let names: Vec<&str> = workers.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["builder", "triager"]);
}

#[test]
fn collect_all_workers_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let workers = collect_all_workers(tmp.path()).unwrap();
    assert!(workers.is_empty());
}

#[test]
fn collect_all_workers_missing_dir() {
    let workers = collect_all_workers(Path::new("/nonexistent")).unwrap();
    assert!(workers.is_empty());
}

#[test]
fn collect_all_workers_skips_invalid() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "bad.hcl", "not valid HCL {{{}}}");
    write_hcl(tmp.path(), "build.hcl", WORKER_RUNBOOK);

    let workers = collect_all_workers(tmp.path()).unwrap();
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0].0, "builder");
}

#[test]
fn collect_all_crons_from_runbook() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "cron.hcl", CRON_RUNBOOK);

    let crons = collect_all_crons(tmp.path()).unwrap();
    assert_eq!(crons.len(), 1);
    assert_eq!(crons[0].0, "daily-backup");
}

#[test]
fn collect_all_crons_empty_dir() {
    let tmp = TempDir::new().unwrap();
    let crons = collect_all_crons(tmp.path()).unwrap();
    assert!(crons.is_empty());
}

#[test]
fn collect_all_crons_missing_dir() {
    let crons = collect_all_crons(Path::new("/nonexistent")).unwrap();
    assert!(crons.is_empty());
}

#[test]
fn collect_all_crons_skips_invalid() {
    let tmp = TempDir::new().unwrap();
    write_hcl(tmp.path(), "bad.hcl", "not valid HCL {{{}}}");
    write_hcl(tmp.path(), "cron.hcl", CRON_RUNBOOK);

    let crons = collect_all_crons(tmp.path()).unwrap();
    assert_eq!(crons.len(), 1);
    assert_eq!(crons[0].0, "daily-backup");
}

// ============================================================================
// extract_block_comments tests
// ============================================================================

#[test]
fn extract_block_comments_multi_command_file() {
    let content = r#"# First command description.
#
# Examples:
#   oj run first

command "first" {
  run = "echo first"
}

# Second command description.
command "second" {
  run = "echo second"
}
"#;
    let comments = extract_block_comments(content);
    assert_eq!(comments.len(), 2);
    assert_eq!(comments["first"].short, "First command description.");
    assert!(comments["first"].long.contains("Examples:"));
    assert_eq!(comments["second"].short, "Second command description.");
    assert!(comments["second"].long.is_empty());
}

#[test]
fn extract_block_comments_no_comment() {
    let content = r#"command "bare" { run = "echo" }"#;
    let comments = extract_block_comments(content);
    assert!(comments.is_empty());
}

#[test]
fn extract_block_comments_ignores_section_separators() {
    // The "# ---" separator should not bleed into the second command's comment
    let content = r#"# First description
command "first" {
  run = "echo"
}

# ------------------------------------------------------------------
# Agents
# ------------------------------------------------------------------

# Second description
command "second" {
  run = "echo"
}
"#;
    let comments = extract_block_comments(content);
    assert_eq!(comments["second"].short, "Second description");
}

#[test]
fn extract_block_comments_blank_lines_between() {
    // Blank lines between comment block and command line are skipped
    let content = r#"# Description here

command "test" {
  run = "echo"
}
"#;
    let comments = extract_block_comments(content);
    assert_eq!(comments.len(), 1);
    assert_eq!(comments["test"].short, "Description here");
}

#[test]
fn collect_all_commands_per_block_descriptions() {
    let tmp = TempDir::new().unwrap();
    let content = r#"# First command
command "alpha" {
  run = "echo alpha"
}

# Second command
command "beta" {
  run = "echo beta"
}
"#;
    write_hcl(tmp.path(), "multi.hcl", content);
    let commands = collect_all_commands(tmp.path()).unwrap();
    let alpha = commands.iter().find(|(n, _, _)| n == "alpha").unwrap();
    let beta = commands.iter().find(|(n, _, _)| n == "beta").unwrap();
    assert_eq!(alpha.2.as_deref(), Some("First command"));
    assert_eq!(beta.2.as_deref(), Some("Second command"));
}

#[test]
fn find_command_with_comment_uses_block_comment() {
    let tmp = TempDir::new().unwrap();
    let content = r#"# File-level comment
# Shared description

# Alpha-specific description
#
# Alpha long details
command "alpha" {
  run = "echo alpha"
}

# Beta-specific description
command "beta" {
  run = "echo beta"
}
"#;
    write_hcl(tmp.path(), "multi.hcl", content);

    let result = find_command_with_comment(tmp.path(), "alpha").unwrap();
    let (cmd, comment) = result.unwrap();
    assert_eq!(cmd.name, "alpha");
    let comment = comment.unwrap();
    assert_eq!(comment.short, "Alpha-specific description");
    assert!(comment.long.contains("Alpha long details"));

    let result = find_command_with_comment(tmp.path(), "beta").unwrap();
    let (cmd, comment) = result.unwrap();
    assert_eq!(cmd.name, "beta");
    let comment = comment.unwrap();
    assert_eq!(comment.short, "Beta-specific description");
}

// ============================================================================
// Imported command comment tests (bug: oj-92dbf159)
// ============================================================================

/// Set up a project dir with runbooks/ and libraries/ for import tests.
/// Returns (runbooks_dir, project_dir).
fn setup_import_project(
    base_hcl: &str,
    library_name: &str,
    library_files: &[(&str, &str)],
) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    // runbooks/ lives inside a project dir (e.g., .oj/)
    let project = tmp.path().join("project");
    let runbooks = project.join("runbooks");
    let libraries = project.join("libraries").join(library_name);
    fs::create_dir_all(&runbooks).unwrap();
    fs::create_dir_all(&libraries).unwrap();
    fs::write(runbooks.join("base.hcl"), base_hcl).unwrap();
    for (name, content) in library_files {
        fs::write(libraries.join(name), content).unwrap();
    }
    (tmp, runbooks)
}

#[test]
fn imported_commands_use_library_comment_not_importer_comment() {
    let base = r#"# Shared imports for the project
import "mylib" {}
"#;
    let lib_cmd = r#"# Library command description
command "deploy" {
  run = "echo deploy"
}
"#;
    let (_tmp, runbooks) = setup_import_project(base, "mylib", &[("cmd.hcl", lib_cmd)]);

    let commands = collect_all_commands(&runbooks).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "deploy");
    // Should use the library's comment, not the importer's "Shared imports" comment
    assert_eq!(
        commands[0].2.as_deref(),
        Some("Library command description")
    );
}

#[test]
fn imported_commands_with_alias_use_library_comment() {
    let base = r#"# Shared imports for the project
import "mylib" { alias = "ml" }
"#;
    let lib_cmd = r#"# Aliased library command
#
# Long description for the aliased command.
command "deploy" {
  run = "echo deploy"
}
"#;
    let (_tmp, runbooks) = setup_import_project(base, "mylib", &[("cmd.hcl", lib_cmd)]);

    let commands = collect_all_commands(&runbooks).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "ml:deploy");
    assert_eq!(commands[0].2.as_deref(), Some("Aliased library command"));
}

#[test]
fn find_command_with_comment_imported_uses_library_comment() {
    let base = r#"# Shared imports
import "mylib" {}
"#;
    let lib_cmd = r#"# Build artifacts
#
# Usage:
#   oj run build <target>
command "build" {
  args = "<target>"
  run  = "echo build"
}
"#;
    let (_tmp, runbooks) = setup_import_project(base, "mylib", &[("build.hcl", lib_cmd)]);

    let result = find_command_with_comment(&runbooks, "build").unwrap();
    assert!(result.is_some());
    let (cmd, comment) = result.unwrap();
    assert_eq!(cmd.name, "build");
    let comment = comment.unwrap();
    assert_eq!(comment.short, "Build artifacts");
    assert!(comment.long.contains("Usage:"));
}

#[test]
fn find_command_with_comment_aliased_import_uses_library_comment() {
    let base = r#"# Shared imports
import "mylib" { alias = "ml" }
"#;
    let lib_cmd = r#"# Plan work interactively
command "plan" {
  run = "echo plan"
}
"#;
    let (_tmp, runbooks) = setup_import_project(base, "mylib", &[("plan.hcl", lib_cmd)]);

    let result = find_command_with_comment(&runbooks, "ml:plan").unwrap();
    assert!(result.is_some());
    let (cmd, comment) = result.unwrap();
    assert_eq!(cmd.name, "ml:plan");
    let comment = comment.unwrap();
    assert_eq!(comment.short, "Plan work interactively");
}

#[test]
fn local_commands_still_use_file_comment_as_fallback() {
    let tmp = TempDir::new().unwrap();
    let content = r#"# File-level description
command "test" {
  run = "echo test"
}
"#;
    write_hcl(tmp.path(), "test.hcl", content);

    let commands = collect_all_commands(tmp.path()).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].2.as_deref(), Some("File-level description"));
}

#[test]
fn mixed_local_and_imported_commands_get_correct_comments() {
    let base = r#"# Local project commands
import "mylib" { alias = "lib" }

# Run local tests
command "test" {
  run = "echo test"
}
"#;
    let lib_cmd = r#"# Deploy to production
command "deploy" {
  run = "echo deploy"
}
"#;
    let (_tmp, runbooks) = setup_import_project(base, "mylib", &[("deploy.hcl", lib_cmd)]);

    let commands = collect_all_commands(&runbooks).unwrap();
    assert_eq!(commands.len(), 2);

    let deploy = commands.iter().find(|(n, _, _)| n == "lib:deploy").unwrap();
    let test = commands.iter().find(|(n, _, _)| n == "test").unwrap();

    // Imported command gets library comment
    assert_eq!(deploy.2.as_deref(), Some("Deploy to production"));
    // Local command gets its block comment
    assert_eq!(test.2.as_deref(), Some("Run local tests"));
}
