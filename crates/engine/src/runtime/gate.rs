// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Gate command execution and error parsing.

use oj_adapters::subprocess::{run_with_timeout, GATE_TIMEOUT};

/// Run a shell gate command. Returns `Ok(())` on exit code 0 or `Err(message)` on failure.
pub(crate) async fn run_gate_command(command: &str, cwd: &std::path::Path) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(command).current_dir(cwd);

    match run_with_timeout(cmd, GATE_TIMEOUT, "gate command").await {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr_trimmed = stderr.trim();
            let error = if stderr_trimmed.is_empty() {
                format!("gate `{}` failed (exit {})", command, exit_code)
            } else {
                format!("gate `{}` failed (exit {}): {}", command, exit_code, stderr_trimmed)
            };
            Err(error)
        }
        Err(e) => Err(format!("gate `{}` execution error: {}", command, e)),
    }
}

/// Parse a gate error string into exit code and stderr.
///
/// The `run_gate_command` function produces errors in the format:
/// - `"gate `cmd` failed (exit N)"` - without stderr
/// - `"gate `cmd` failed (exit N): stderr_content"` - with stderr
/// - `"gate `cmd` execution error: msg"` - for spawn failures
pub(crate) fn parse_gate_error(error: &str) -> (i32, String) {
    // Try to extract exit code from "(exit N)" pattern
    if let Some(exit_start) = error.find("(exit ") {
        let after_exit = &error[exit_start + 6..];
        if let Some(paren_end) = after_exit.find(')') {
            if let Ok(code) = after_exit[..paren_end].trim().parse::<i32>() {
                // Check if there's stderr after the closing paren
                let rest = &after_exit[paren_end + 1..];
                let stderr = if let Some(colon_pos) = rest.find(':') {
                    rest[colon_pos + 1..].trim().to_string()
                } else {
                    String::new()
                };
                return (code, stderr);
            }
        }
    }
    // Fallback: unknown exit code, full string as stderr
    (1, error.to_string())
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
