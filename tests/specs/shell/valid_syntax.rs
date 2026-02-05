//! Tests for valid shell syntax acceptance.
//!
//! Verify runbooks with complex shell commands pass validation.

use crate::prelude::*;

/// Simple echo command
#[test]
fn accepts_simple_command() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "echo 'hello world'"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Job with multiple commands
#[test]
fn accepts_job_commands() {
    let temp = Project::empty();
    temp.git_init();
    temp.file("README.md", "# Readme file");
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "cat README.md | grep -i readme | wc -l"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Logical operators (&&, ||)
#[test]
fn accepts_logical_operators() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "test -f README.md && cat README.md || echo 'not found'"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Subshell syntax
#[test]
fn accepts_subshell() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "(cd /tmp && pwd)"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Brace group syntax
#[test]
fn accepts_brace_group() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "{ echo start; echo end; }"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Redirections
#[test]
fn accepts_redirections() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "echo output > /tmp/test.txt 2>&1"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Environment variable assignment
#[test]
fn accepts_env_assignment() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "FOO=bar BAZ=qux env | grep FOO"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Variable substitution
#[test]
fn accepts_variable_substitution() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "echo $HOME ${USER:-default}"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Command substitution
#[test]
fn accepts_command_substitution() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "echo \"Today is $(date +%Y-%m-%d)\""
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Multi-line shell command (triple quotes)
#[test]
fn accepts_multiline_command() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = """
echo 'line 1'
echo 'line 2'
echo 'line 3'
"""
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Background command (&)
#[test]
fn accepts_background_command() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "sleep 0.1 & echo 'started'"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Nested subshells
#[test]
fn accepts_nested_subshells() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "((echo deep))"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Escaped characters in strings
#[test]
fn accepts_escaped_characters() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "echo \"quoted \\\"string\\\" here\""
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Here-string syntax
#[test]
fn accepts_here_string() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "cat <<< 'here string'"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}

/// Semicolon-separated commands
#[test]
fn accepts_semicolon_separated() {
    let temp = Project::empty();
    temp.git_init();
    temp.file(
        ".oj/runbooks/test.toml",
        r#"
[command.test]
run = { job = "test" }

[job.test]

[[job.test.step]]
name = "run"
run = "echo one; echo two; echo three"
"#,
    );
    temp.oj().args(&["daemon", "start"]).passes();
    temp.oj().args(&["run", "test"]).passes();
}
