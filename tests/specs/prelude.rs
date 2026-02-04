//! Test helpers for behavioral specifications.
//!
//! Provides high-level DSL for testing oj CLI behavior.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

// Aggressive timeouts for fast tests.
//
// IMPORTANT:
//   Do NOT change these.
//   File a performance bug instead.
const OJ_TIMEOUT_CONNECT_MS: &str = "2000";
const OJ_TIMEOUT_EXIT_MS: &str = "500";
const OJ_TIMEOUT_IPC_MS: &str = "500";
const OJ_CONNECT_POLL_MS: &str = "5";
const OJ_SESSION_POLL_MS: &str = "50";
const OJ_WATCHER_POLL_MS: &str = "500";
const OJ_PROMPT_POLL_MS: &str = "200"; // 200ms (1 check) - tests use trusted=true so no prompt expected

// Spec polling timeouts
pub const SPEC_POLL_INTERVAL_MS: u64 = 10;
pub const SPEC_WAIT_MAX_MS: u64 = 2000;

/// Returns the path to a binary, checking llvm-cov target directory first.
/// This works with both standard builds and llvm-cov coverage runs.
/// Falls back to resolving relative to the test binary itself when
/// CARGO_MANIFEST_DIR is stale (e.g. compiled by a removed worktree
/// into a shared target directory).
fn binary_path(name: &str) -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Check for llvm-cov target directory first
    let llvm_cov_path = manifest_dir.join("target/llvm-cov-target/debug").join(name);
    if llvm_cov_path.exists() {
        return llvm_cov_path;
    }

    // Standard target directory (works when CARGO_MANIFEST_DIR is correct)
    let standard = manifest_dir.join("target/debug").join(name);
    if standard.exists() {
        return standard;
    }

    // Fallback: resolve relative to the test binary itself.
    // The test binary lives at target/debug/deps/specs-<hash>, so its
    // grandparent is target/debug/ where oj and ojd are built.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(debug_dir) = exe.parent().and_then(|d| d.parent()) {
            let fallback = debug_dir.join(name);
            if fallback.exists() {
                return fallback;
            }
        }
    }

    standard
}

/// Returns the path to the oj binary.
fn oj_binary() -> PathBuf {
    binary_path("oj")
}

/// Returns the path to the ojd daemon binary.
pub fn ojd_binary() -> PathBuf {
    binary_path("ojd")
}

/// Returns a Command configured to run the oj binary
pub fn oj_cmd() -> Command {
    Command::new(oj_binary())
}

/// Create a CLI builder for oj commands
pub fn cli() -> CliBuilder {
    CliBuilder::new()
}

/// High-level CLI builder for fluent test assertions
pub struct CliBuilder {
    args: Vec<String>,
    dir: Option<PathBuf>,
    envs: Vec<(String, String)>,
}

impl CliBuilder {
    fn new() -> Self {
        Self {
            args: Vec::new(),
            dir: None,
            envs: vec![
                (
                    "OJ_DAEMON_BINARY".into(),
                    ojd_binary().to_string_lossy().into(),
                ),
                ("OJ_TIMEOUT_CONNECT_MS".into(), OJ_TIMEOUT_CONNECT_MS.into()),
                ("OJ_TIMEOUT_EXIT_MS".into(), OJ_TIMEOUT_EXIT_MS.into()),
                ("OJ_TIMEOUT_IPC_MS".into(), OJ_TIMEOUT_IPC_MS.into()),
                ("OJ_CONNECT_POLL_MS".into(), OJ_CONNECT_POLL_MS.into()),
                ("OJ_SESSION_POLL_MS".into(), OJ_SESSION_POLL_MS.into()),
                ("OJ_WATCHER_POLL_MS".into(), OJ_WATCHER_POLL_MS.into()),
                ("OJ_PROMPT_POLL_MS".into(), OJ_PROMPT_POLL_MS.into()),
            ],
        }
    }

    /// Add CLI arguments
    pub fn args(mut self, args: &[&str]) -> Self {
        self.args.extend(args.iter().map(|s| s.to_string()));
        self
    }

    /// Set working directory
    pub fn pwd(mut self, path: impl Into<PathBuf>) -> Self {
        self.dir = Some(path.into());
        self
    }

    /// Set environment variable
    pub fn env(mut self, key: &str, value: impl AsRef<Path>) -> Self {
        self.envs.push((
            key.to_string(),
            value.as_ref().to_string_lossy().to_string(),
        ));
        self
    }

    /// Build the command without running it
    pub fn command(self) -> Command {
        let mut cmd = oj_cmd();
        cmd.args(&self.args);

        if let Some(dir) = self.dir {
            cmd.current_dir(dir);
        }

        // Prevent parent OJ_NAMESPACE from leaking into tests.
        // It overrides auto-resolved namespace, which would scope
        // operations (e.g. pipeline run) to the wrong project.
        cmd.env_remove("OJ_NAMESPACE");

        for (key, value) in self.envs {
            cmd.env(key, value);
        }

        cmd
    }

    /// Run and expect success (exit code 0)
    pub fn passes(self) -> RunAssert {
        let mut cmd = self.command();
        let output = cmd.output().expect("command should run");
        assert!(
            output.status.success(),
            "expected command to pass, got exit code {:?}\nstdout: {}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        RunAssert { output }
    }

    /// Run and expect failure (non-zero exit code)
    pub fn fails(self) -> RunAssert {
        let mut cmd = self.command();
        let output = cmd.output().expect("command should run");
        assert!(
            !output.status.success(),
            "expected command to fail, but it passed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        RunAssert { output }
    }
}

/// Result of a CLI run for chaining assertions
pub struct RunAssert {
    output: Output,
}

impl RunAssert {
    /// Get stdout as string
    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }

    /// Get stderr as string
    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }

    /// Assert stdout equals expected exactly (with diff on failure).
    /// **Prefer this for format specs** - catches format regressions.
    pub fn stdout_eq(self, expected: &str) -> Self {
        let stdout = self.stdout();
        similar_asserts::assert_eq!(stdout, expected);
        self
    }

    /// Assert stderr equals expected exactly (with diff on failure).
    pub fn stderr_eq(self, expected: &str) -> Self {
        let stderr = self.stderr();
        similar_asserts::assert_eq!(stderr, expected);
        self
    }

    /// Assert stdout contains substring.
    /// Use when exact comparison isn't practical.
    pub fn stdout_has(self, expected: &str) -> Self {
        let stdout = self.stdout();
        assert!(
            stdout.contains(expected),
            "stdout does not contain '{}'\nstdout: {}",
            expected,
            stdout
        );
        self
    }

    /// Assert stdout does not contain substring.
    pub fn stdout_lacks(self, unexpected: &str) -> Self {
        let stdout = self.stdout();
        assert!(
            !stdout.contains(unexpected),
            "stdout should not contain '{}'\nstdout: {}",
            unexpected,
            stdout
        );
        self
    }

    /// Assert stderr contains substring.
    pub fn stderr_has(self, expected: &str) -> Self {
        let stderr = self.stderr();
        assert!(
            stderr.contains(expected),
            "stderr does not contain '{}'\nstderr: {}",
            expected,
            stderr
        );
        self
    }

    /// Assert stderr does not contain substring.
    pub fn stderr_lacks(self, unexpected: &str) -> Self {
        let stderr = self.stderr();
        assert!(
            !stderr.contains(unexpected),
            "stderr should not contain '{}'\nstderr: {}",
            unexpected,
            stderr
        );
        self
    }
}

// =============================================================================
// Polling
// =============================================================================

/// Poll a condition until it returns true or timeout is reached.
/// Uses aggressive polling for fast tests.
pub fn wait_for<F>(timeout_ms: u64, mut condition: F) -> bool
where
    F: FnMut() -> bool,
{
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let poll_interval = std::time::Duration::from_millis(SPEC_POLL_INTERVAL_MS);

    while start.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(poll_interval);
    }
    false
}

// =============================================================================
// Project
// =============================================================================

/// Temporary test project directory with helper methods.
pub struct Project {
    dir: tempfile::TempDir,
    /// Isolated state directory for this test (XDG_STATE_HOME)
    state_dir: tempfile::TempDir,
}

impl Project {
    /// Create an empty project
    pub fn empty() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            state_dir: tempfile::tempdir().unwrap(),
        }
    }

    /// Get the project path
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Initialize git repository
    pub fn git_init(&self) {
        Command::new("git")
            .args(["init"])
            .current_dir(self.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git init should work");
    }

    /// Write a file at the given path (parent directories created automatically)
    pub fn file(&self, path: impl AsRef<Path>, content: &str) {
        let full_path = self.dir.path().join(path.as_ref());
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full_path, content).unwrap();
    }

    /// Get the isolated state directory path
    pub fn state_path(&self) -> &Path {
        self.state_dir.path()
    }

    /// Run oj command in this project's context
    pub fn oj(&self) -> CliBuilder {
        cli()
            .pwd(self.path())
            .env("OJ_STATE_DIR", self.state_path())
            // Set CLAUDE_CONFIG_DIR so claudeless writes JSONL where the watcher
            // expects it. Without this, claudeless defaults to a temp dir while
            // the watcher defaults to ~/.claude, and they never find each other.
            .env("CLAUDE_CONFIG_DIR", self.state_path().join("claude"))
    }

    /// Read the daemon log file contents (for debugging test failures)
    pub fn daemon_log(&self) -> String {
        let log_path = self.state_path().join("daemon.log");
        std::fs::read_to_string(&log_path).unwrap_or_else(|_| "(no daemon log)".to_string())
    }

    /// Kill the daemon process with SIGKILL (simulates crash).
    /// Returns true if the process was killed, false if PID not found or kill failed.
    pub fn daemon_kill(&self) -> bool {
        let pid_file = self.state_path().join("daemon.pid");
        if let Ok(content) = std::fs::read_to_string(&pid_file) {
            if let Ok(pid) = content.trim().parse::<u32>() {
                Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            } else {
                false
            }
        } else {
            false
        }
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        // Always try to stop daemon (no-op if not running)
        let mut cmd = self.oj().args(&["daemon", "stop", "--kill"]).command();
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let _ = cmd.status();
    }
}

/// Minimal runbook for testing
pub const MINIMAL_RUNBOOK: &str = r#"
[command.build]
args = "<name> <prompt>"
run = { pipeline = "build" }

[pipeline.build]
vars  = ["name", "prompt"]

[[pipeline.build.step]]
name = "execute"
run = "echo 'Building ${name}'"
"#;
