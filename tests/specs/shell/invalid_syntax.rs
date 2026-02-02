//! Tests for invalid shell syntax rejection.
//!
//! Verify runbooks with malformed shell commands fail with clear errors.

use crate::prelude::*;

/// Unterminated single quote
#[test]
fn rejects_unterminated_single_quote() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "echo 'unterminated"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "test"])
        .fails()
        .stderr_has("unterminated");
}

/// Unterminated double quote
#[test]
fn rejects_unterminated_double_quote() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "echo \"unterminated"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "test"])
        .fails()
        .stderr_has("unterminated");
}

/// Unterminated subshell
#[test]
fn rejects_unterminated_subshell() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "(echo hello"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "test"])
        .fails()
        .stderr_has("expected"); // Expected ')' or similar
}

/// Unterminated brace group
#[test]
fn rejects_unterminated_brace_group() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "{ echo hello"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "test"])
        .fails()
        .stderr_has("expected"); // Expected '}' or similar
}

/// Dangling pipe
#[test]
fn rejects_dangling_pipe() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "echo hello |"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "test"])
        .fails()
        .stderr_has("unexpected"); // Unexpected EOF or similar
}

/// Dangling && operator
#[test]
fn rejects_dangling_and_operator() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "echo hello &&"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "test"])
        .fails()
        .stderr_has("unexpected");
}

/// Dangling || operator
#[test]
fn rejects_dangling_or_operator() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "echo hello ||"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "test"])
        .fails()
        .stderr_has("unexpected");
}

/// Invalid redirection (no target)
#[test]
fn rejects_redirection_without_target() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "echo hello >"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "test"])
        .fails()
        .stderr_has("unexpected");
}

/// Unterminated command substitution
#[test]
fn rejects_unterminated_command_substitution() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "echo $(date"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "test"])
        .fails()
        .stderr_has("unterminated");
}

/// Empty subshell (semantic validation)
#[test]
fn rejects_empty_subshell() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "( )"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).fails().stderr_has("empty");
}

/// Empty brace group (semantic validation)
#[test]
fn rejects_empty_brace_group() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "run"
run = "{ }"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).fails().stderr_has("empty");
}

/// Error message includes step name for context
#[test]
fn error_includes_step_context() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { pipeline = "test" }

[pipeline.test]

[[pipeline.test.step]]
name = "my_broken_step"
run = "echo 'unterminated"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj()
        .args(&["run", "test"])
        .fails()
        .stderr_has("my_broken_step");
}
