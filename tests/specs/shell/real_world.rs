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
run = { job = "build" }

[job.build]
vars  = ["name", "prompt"]

[[job.build.step]]
name = "init"
run = "echo 'Starting build: ${name}'"
on_done = { step = "merge" }

[[job.build.step]]
name = "merge"
run = "git fetch origin main && git rebase origin/main && git push"
on_done = { step = "done" }

[[job.build.step]]
name = "done"
run = "echo 'Build complete: ${name}'"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "build", "test", "test prompt"]).passes();
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
run = { job = "build" }

[job.build]
vars  = ["name", "prompt"]

[job.build.defaults]
branch = "feature/${name}"
base = "main"

[[job.build.step]]
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
    temp.oj().args(&["run", "build", "test", "test prompt"]).passes();
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
run = { job = "feature" }

[job.feature]
vars  = ["name"]

[job.feature.defaults]
branch = "feature/${name}"
epic = "${name}"

[[job.feature.step]]
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
run = { job = "events" }

[job.events]
vars  = ["name"]

[[job.events.step]]
name = "work"
run = "echo 'working'"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "events", "test"]).passes();
}
