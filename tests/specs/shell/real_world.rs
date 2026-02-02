//! Tests using real-world runbook examples.
//!
//! Verify actual runbooks from docs/10-runbooks/ pass validation.

use crate::prelude::*;

/// Test build.minimal.toml syntax is valid
#[test]
fn build_minimal_runbook_is_valid() {
    let temp = Project::empty();
    temp.git_init();
    // Use shell commands from build.minimal.toml
    temp.file(
        ".oj/runbooks/build.toml",
        r#"
[command.build]
args = "<name> <prompt>"
run = { pipeline = "build" }

[pipeline.build]
vars  = ["name", "prompt"]

[[pipeline.build.step]]
name = "init"
run = "echo 'Starting build: ${name}'"

[[pipeline.build.step]]
name = "merge"
run = "git fetch origin main && git rebase origin/main && git push"

[[pipeline.build.step]]
name = "done"
run = "echo 'Build complete: ${name}'"

[pipeline.build.events]
on_step = "echo '${name} -> ${step}' >> .oj/build.log"
on_complete = "echo '${name} complete' >> .oj/build.log"
on_fail = "echo '${name} failed: ${error}' >> .oj/build.log"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "build", "test", "test prompt"])
        .passes();
}

/// Test merge step with remote guard is valid shell
#[test]
fn merge_with_remote_guard_is_valid() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/build.toml",
        r#"
[command.build]
args = "<name> <prompt>"
run = { pipeline = "build" }

[pipeline.build]
vars  = ["name", "prompt"]

[pipeline.build.defaults]
branch = "feature/${name}"
base = "main"

[[pipeline.build.step]]
name = "merge"
run = """
git remote | grep -q . || exit 0
git fetch origin ${base}
git rebase origin/${base}
git push origin HEAD:${branch}
"""
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "build", "test", "test prompt"])
        .passes();
}

/// Test guard conditions are valid shell
#[test]
fn guard_conditions_are_valid() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/guarded.toml",
        r#"
[command.guarded]
args = "<name>"
run = { pipeline = "guarded" }

[pipeline.guarded]
vars  = ["name"]

[[pipeline.guarded.step]]
name = "run"
pre = ["file_exists"]
run = "cat data.txt"

[guard.file_exists]
condition = "test -f data.txt"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    // Pipeline starts (guard will block, but syntax is valid)
    temp.oj().args(&["run", "guarded", "test"]).passes();
}

/// Test complex guard with pipes and grep
#[test]
fn complex_guard_condition_is_valid() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/complex.toml",
        r#"
[command.complex]
args = "<name> <after>"
run = { pipeline = "complex" }

[pipeline.complex]
vars  = ["name", "after"]

[pipeline.complex.defaults]
after = ""

[[pipeline.complex.step]]
name = "blocked"
pre = ["blocker_merged"]
run = "echo 'unblocked'"

[guard.blocker_merged]
condition = "test -z '${after}' || oj pipeline show ${after} --step | grep -q done"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "complex", "test", ""]).passes();
}

/// Test multi-line done step with delete and wok
#[test]
fn multiline_done_step_is_valid() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/feature.toml",
        r#"
[command.feature]
args = "<name>"
run = { pipeline = "feature" }

[pipeline.feature]
vars  = ["name"]

[pipeline.feature.defaults]
branch = "feature/${name}"
epic = "${name}"

[[pipeline.feature.step]]
name = "done"
run = """
git push origin --delete ${branch}
echo 'Feature complete: ${name}'
"""
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "feature", "test"]).passes();
}

/// Test event handlers with variable interpolation
#[test]
fn event_handlers_are_valid() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/events.toml",
        r#"
[command.events]
args = "<name>"
run = { pipeline = "events" }

[pipeline.events]
vars  = ["name"]

[[pipeline.events.step]]
name = "work"
run = "echo 'working'"

[pipeline.events.events]
on_step = "oj emit pipeline:advanced --id ${name} --step ${step}"
on_complete = "oj emit pipeline:complete --id ${name}"
on_fail = "oj emit pipeline:fail --id ${name} --error '${error}'"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "events", "test"]).passes();
}
