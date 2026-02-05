// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Background agent watcher using file notifications
//!
//! Watches Claude's session log file and emits events when the agent's state changes.

use crate::agent::log_entry::{self, AgentLogMessage};
use crate::session::SessionAdapter;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use oj_core::{AgentError, AgentId, AgentState, Event};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Configuration for the agent watcher
pub struct WatcherConfig {
    pub agent_id: AgentId,
    /// Session ID for log file lookup (matches --session-id passed to claude)
    pub log_session_id: String,
    /// Session ID for tmux liveness checks (includes oj- prefix)
    pub tmux_session_id: String,
    pub project_path: PathBuf,
    pub process_name: String,
}

/// Get the watcher fallback poll interval.
/// Configurable via `OJ_WATCHER_POLL_MS` env var (default: 5000ms).
fn watcher_poll_interval() -> Duration {
    crate::env::watcher_poll_ms()
}

/// Get the session log poll interval for `wait_for_session_log_or_exit`.
/// Configurable via `OJ_SESSION_POLL_MS` env var (default: 1000ms).
fn session_log_poll_interval() -> Duration {
    crate::env::session_poll_ms()
}

/// Start watching an agent's session log
///
/// Spawns a background task that monitors the session log file and emits
/// `AgentStateChanged` events when the state changes. Also performs periodic
/// process liveness checks as a fallback.
///
/// Returns a shutdown sender that can be used to stop the watcher.
pub fn start_watcher<S: SessionAdapter>(
    config: WatcherConfig,
    sessions: S,
    event_tx: mpsc::Sender<Event>,
    log_entry_tx: Option<mpsc::Sender<AgentLogMessage>>,
) -> oneshot::Sender<()> {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    tokio::spawn(watch_agent(
        config,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx,
    ));

    shutdown_tx
}

async fn watch_agent<S: SessionAdapter>(
    config: WatcherConfig,
    sessions: S,
    event_tx: mpsc::Sender<Event>,
    shutdown_rx: oneshot::Receiver<()>,
    log_entry_tx: Option<mpsc::Sender<AgentLogMessage>>,
) {
    let WatcherConfig {
        agent_id,
        log_session_id,
        tmux_session_id,
        project_path,
        process_name,
    } = config;
    // Track spawn-to-first-log duration
    let spawn_time = std::time::Instant::now();

    // Find the session log path (may take a moment to be created).
    // Also check session liveness so we detect early exits without
    // waiting the full 30 seconds.
    // Use log_session_id for log lookup, tmux_session_id for liveness check
    let log_path = match wait_for_session_log_or_exit(
        &project_path,
        &log_session_id,
        &tmux_session_id,
        &sessions,
    )
    .await
    {
        SessionLogWait::Found(path) => {
            let elapsed = spawn_time.elapsed();
            tracing::info!(
                agent_id = %agent_id,
                log_session_id,
                elapsed_ms = elapsed.as_millis() as u64,
                "session log found (spawn-to-first-log)"
            );
            path
        }
        SessionLogWait::SessionDied => {
            tracing::info!(
                agent_id = %agent_id,
                log_session_id,
                "session ended while waiting for log"
            );
            let _ = event_tx.send(Event::AgentGone { agent_id }).await;
            return;
        }
        SessionLogWait::Timeout => {
            tracing::warn!(
                agent_id = %agent_id,
                log_session_id,
                project_path = %project_path.display(),
                "session log not found, using fallback polling"
            );
            // Fall back to process-only monitoring (no agent log extraction)
            poll_process_only(
                agent_id,
                tmux_session_id,
                process_name,
                sessions,
                event_tx,
                shutdown_rx,
            )
            .await;
            return;
        }
    };

    tracing::debug!(
        agent_id = %agent_id,
        log_path = %log_path.display(),
        "starting file watcher"
    );

    // Set up file watcher
    let (file_tx, file_rx) = mpsc::channel(32);
    let watcher_result = create_file_watcher(&log_path, file_tx);

    let _watcher_guard = match watcher_result {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(
                agent_id = %agent_id,
                error = %e,
                "failed to create file watcher, using fallback polling"
            );
            poll_process_only(
                agent_id,
                tmux_session_id,
                process_name,
                sessions,
                event_tx,
                shutdown_rx,
            )
            .await;
            return;
        }
    };

    watch_loop(WatchLoopParams {
        agent_id,
        tmux_session_id,
        process_name,
        log_path,
        sessions,
        event_tx,
        shutdown_rx,
        log_entry_tx,
        file_rx,
    })
    .await;
}

struct WatchLoopParams<S> {
    agent_id: AgentId,
    tmux_session_id: String,
    process_name: String,
    log_path: PathBuf,
    sessions: S,
    event_tx: mpsc::Sender<Event>,
    shutdown_rx: oneshot::Receiver<()>,
    log_entry_tx: Option<mpsc::Sender<AgentLogMessage>>,
    file_rx: mpsc::Receiver<()>,
}

async fn watch_loop<S: SessionAdapter>(params: WatchLoopParams<S>) {
    let WatchLoopParams {
        agent_id,
        tmux_session_id,
        process_name,
        log_path,
        sessions,
        event_tx,
        mut shutdown_rx,
        log_entry_tx,
        mut file_rx,
    } = params;

    // Check initial state - agent may have become idle while daemon was down
    let mut parser = SessionLogParser::new();
    let initial_state = parser.parse(&log_path);
    let mut last_state = initial_state.clone();
    let mut last_log_offset: u64 = 0;

    // Emit non-working state immediately so on_dead fires after reconnect.
    // WaitingForInput is emitted as AgentIdle (same event the Notification
    // hook produces) so the on_idle handler fires without a timeout delay.
    if initial_state == AgentState::WaitingForInput {
        tracing::info!(
            agent_id = %agent_id,
            "initial state is idle, emitting AgentIdle immediately"
        );
        let _ = event_tx
            .send(Event::AgentIdle {
                agent_id: agent_id.clone(),
            })
            .await;
    } else if initial_state != AgentState::Working {
        tracing::info!(
            agent_id = %agent_id,
            state = ?initial_state,
            "initial state is non-working, emitting immediately"
        );
        let _ = event_tx
            .send(Event::from_agent_state(agent_id.clone(), initial_state))
            .await;
    }

    loop {
        tokio::select! {
            // File changed
            Some(_) = file_rx.recv() => {
                let new_state = parser.parse(&log_path);

                if new_state != last_state {
                    last_state = new_state.clone();

                    // WaitingForInput is emitted as AgentIdle (same event the
                    // Notification hook produces) for instant idle detection
                    // without the old timeout delay.
                    if new_state == AgentState::WaitingForInput {
                        let _ = event_tx
                            .send(Event::AgentIdle {
                                agent_id: agent_id.clone(),
                            })
                            .await;
                    } else {
                        let _ = event_tx
                            .send(Event::from_agent_state(agent_id.clone(), new_state))
                            .await;
                    }
                }

                // Extract log entries
                if let Some(ref tx) = log_entry_tx {
                    let (entries, new_offset) = log_entry::parse_entries_from(&log_path, last_log_offset);
                    if !entries.is_empty() {
                        let _ = tx.send((agent_id.clone(), entries)).await;
                    }
                    last_log_offset = new_offset;
                }
            }

            // Periodic check for process death
            _ = tokio::time::sleep(watcher_poll_interval()) => {
                if let Some(state) = check_liveness(&sessions, &tmux_session_id, &process_name, &agent_id).await {
                    let _ = event_tx.send(Event::from_agent_state(agent_id.clone(), state)).await;
                    break;
                }
            }

            // Shutdown
            _ = &mut shutdown_rx => {
                tracing::debug!(agent_id = %agent_id, "watcher shutdown requested");
                break;
            }
        }
    }
}

/// Fallback polling when file watcher isn't available
async fn poll_process_only<S: SessionAdapter>(
    agent_id: AgentId,
    session_id: String,
    process_name: String,
    sessions: S,
    event_tx: mpsc::Sender<Event>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut poll_count: u64 = 0;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(watcher_poll_interval()) => {
                if let Some(state) = check_liveness(&sessions, &session_id, &process_name, &agent_id).await {
                    let _ = event_tx.send(Event::from_agent_state(agent_id.clone(), state)).await;
                    break;
                }
                poll_count += 1;
                if poll_count.is_multiple_of(6) {
                    tracing::debug!(
                        agent_id = %agent_id,
                        session_id,
                        poll_count,
                        "fallback polling: session alive (file-based monitoring not active)"
                    );
                } else {
                    tracing::trace!(
                        agent_id = %agent_id,
                        session_id,
                        poll_count,
                        "fallback polling: session alive"
                    );
                }
            }

            _ = &mut shutdown_rx => break,
        }
    }
}

/// Check whether an agent's session and process are still alive.
///
/// Returns `Some(state)` if the agent has terminated (session gone or process
/// exited), or `None` if everything is still running.
async fn check_liveness<S: SessionAdapter>(
    sessions: &S,
    session_id: &str,
    process_name: &str,
    agent_id: &AgentId,
) -> Option<AgentState> {
    match sessions.is_alive(session_id).await {
        Ok(false) | Err(_) => {
            // Try to get exit code before reporting session gone
            let exit_code = sessions.get_exit_code(session_id).await.ok().flatten();
            tracing::info!(
                agent_id = %agent_id,
                session_id,
                exit_code = ?exit_code,
                "tmux session gone"
            );
            Some(AgentState::SessionGone)
        }
        Ok(true) => match sessions.is_process_running(session_id, process_name).await {
            Ok(false) => {
                // Get exit code from tmux
                let exit_code = sessions.get_exit_code(session_id).await.ok().flatten();
                tracing::info!(
                    agent_id = %agent_id,
                    session_id,
                    process_name,
                    exit_code = ?exit_code,
                    "agent process exited (tmux still open)"
                );
                Some(AgentState::Exited { exit_code })
            }
            Ok(true) => None,
            Err(e) => {
                tracing::warn!(
                    agent_id = %agent_id,
                    session_id,
                    error = %e,
                    "failed to check agent process"
                );
                None
            }
        },
    }
}

enum SessionLogWait {
    /// Session log file was found.
    Found(PathBuf),
    /// The session died before the log appeared.
    SessionDied,
    /// Timed out waiting (30 seconds).
    Timeout,
}

/// Patterns that indicate a trust prompt is being displayed.
const TRUST_PROMPT_PATTERNS: &[&str] = &["Do you trust the files in this folder?", "Do you trust"];

/// Check pane output for trust prompt and auto-accept if found.
///
/// Returns true if a trust prompt was detected and accepted.
async fn check_and_accept_trust_prompt<S: SessionAdapter>(
    sessions: &S,
    tmux_session_id: &str,
) -> bool {
    // Capture recent pane output
    let output = match sessions.capture_output(tmux_session_id, 20).await {
        Ok(output) => output,
        Err(_) => return false,
    };

    // Check for trust prompt patterns
    let has_trust_prompt = TRUST_PROMPT_PATTERNS
        .iter()
        .any(|pattern| output.contains(pattern));

    if has_trust_prompt {
        tracing::info!(tmux_session_id, "detected trust prompt, auto-accepting");
        // Send "y" to accept trust (claudeless handles Y/y as direct accept)
        if let Err(e) = sessions.send(tmux_session_id, "y").await {
            tracing::warn!(
                tmux_session_id,
                error = %e,
                "failed to send trust acceptance"
            );
            return false;
        }
        return true;
    }

    false
}

/// Wait for session log to be created (up to 30 seconds), while also
/// checking session liveness so we detect early agent exits immediately
/// instead of blocking the full 30 seconds.
///
/// Also checks for trust prompts and auto-accepts them so the agent can
/// proceed in untrusted directories.
///
/// # Parameters
/// - `log_session_id`: Session ID for log file lookup (matches --session-id passed to claude)
/// - `tmux_session_id`: Session ID for tmux liveness checks
async fn wait_for_session_log_or_exit<S: SessionAdapter>(
    project_path: &Path,
    log_session_id: &str,
    tmux_session_id: &str,
    sessions: &S,
) -> SessionLogWait {
    for i in 0..30 {
        if let Some(path) = find_session_log(project_path, log_session_id) {
            return SessionLogWait::Found(path);
        }

        // Check session liveness every iteration so we detect dead
        // sessions within ~1s instead of waiting the full 30s.
        if let Ok(false) = sessions.is_alive(tmux_session_id).await {
            tracing::debug!(
                log_session_id,
                tmux_session_id,
                iteration = i,
                "session died while waiting for log"
            );
            return SessionLogWait::SessionDied;
        }

        // Check for trust prompt and auto-accept (first few iterations only)
        if i < 5 {
            check_and_accept_trust_prompt(sessions, tmux_session_id).await;
        }

        tokio::time::sleep(session_log_poll_interval()).await;
    }

    let expected_dir = std::env::var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".claude"))
        .join("projects")
        .join(project_dir_name(project_path));
    tracing::warn!(
        log_session_id,
        expected_path = %expected_dir.join(format!("{log_session_id}.jsonl")).display(),
        "gave up waiting for session log after 30s"
    );

    SessionLogWait::Timeout
}

fn create_file_watcher(
    path: &Path,
    tx: mpsc::Sender<()>,
) -> Result<RecommendedWatcher, notify::Error> {
    let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if res.is_ok() {
            let _ = tx.blocking_send(());
        }
    })?;

    watcher.watch(path, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

/// Incremental parser for Claude's JSONL session log.
///
/// Tracks the last-read byte offset to avoid re-reading the entire file
/// on each invocation. Only new bytes appended since the previous parse
/// are read, similar to how `tail -f` works.
struct SessionLogParser {
    last_offset: u64,
    last_line: String,
}

impl SessionLogParser {
    fn new() -> Self {
        Self {
            last_offset: 0,
            last_line: String::new(),
        }
    }

    /// Parse the session log incrementally, reading only content appended
    /// since the last call.
    fn parse(&mut self, path: &Path) -> AgentState {
        let Ok(file) = File::open(path) else {
            return AgentState::Working; // File not ready yet
        };

        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);

        // If file was truncated (rewritten), reset to beginning
        if file_len < self.last_offset {
            self.last_offset = 0;
            self.last_line.clear();
        }

        // No new content — return state from cached last line
        if file_len == self.last_offset {
            return parse_state_from_line(&self.last_line);
        }

        // Seek to where we left off
        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(self.last_offset)).is_err() {
            return parse_state_from_line(&self.last_line);
        }

        let mut current_offset = self.last_offset;
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    let is_complete = line.ends_with('\n');
                    // Only advance offset for complete lines so incomplete
                    // lines are re-read on the next call.
                    if is_complete {
                        current_offset += n as u64;
                    }

                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        if !is_complete {
                            break;
                        }
                        continue;
                    }

                    // Always update last_line so we return the correct state
                    // even for the final line at EOF without a trailing newline.
                    self.last_line.clear();
                    self.last_line.push_str(trimmed);

                    if !is_complete {
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        self.last_offset = current_offset;
        parse_state_from_line(&self.last_line)
    }
}

/// Parse Claude's session log to determine current state.
///
/// Reads the entire file from the beginning. For repeated calls on a
/// growing log, prefer [`SessionLogParser`] which tracks byte offsets.
pub fn parse_session_log(path: &Path) -> AgentState {
    let mut parser = SessionLogParser::new();
    parser.parse(path)
}

/// Determine agent state from a single JSONL line.
fn parse_state_from_line(line: &str) -> AgentState {
    if line.is_empty() {
        return AgentState::Working;
    }

    // Parse line as JSON
    let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
        return AgentState::Working;
    };

    // Check for error indicators first
    if let Some(failure) = detect_error(&json) {
        return AgentState::Failed(failure);
    }

    let last_type = json.get("type").and_then(|v| v.as_str());

    // Check stop_reason and content for assistant messages
    if last_type == Some("assistant") {
        let stop_reason = json.get("message").and_then(|m| m.get("stop_reason"));

        // If stop_reason is non-null
        if let Some(sr) = stop_reason {
            if !sr.is_null() {
                tracing::warn!(stop_reason = ?sr, "unexpected non-null stop_reason, assuming working");
                return AgentState::Working;
            }
        }

        // stop_reason is null (normal) - check content for active blocks
        // Both tool_use and thinking blocks indicate the agent is actively working
        let has_active_block = json
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter().any(|item| {
                    matches!(
                        item.get("type").and_then(|v| v.as_str()),
                        Some("tool_use") | Some("thinking")
                    )
                })
            })
            .unwrap_or(false);

        return if has_active_block {
            AgentState::Working
        } else {
            AgentState::WaitingForInput
        };
    }

    // User messages mean Claude is working on them (processing tool results)
    if last_type == Some("user") {
        return AgentState::Working;
    }

    AgentState::Working
}

fn detect_error(json: &serde_json::Value) -> Option<AgentError> {
    let error_msg = json.get("error").and_then(|v| v.as_str()).or_else(|| {
        json.get("message")
            .and_then(|m| m.get("error"))
            .and_then(|v| v.as_str())
    });

    if let Some(err) = error_msg {
        let err_lower = err.to_lowercase();
        if err_lower.contains("unauthorized") || err_lower.contains("invalid api key") {
            return Some(AgentError::Unauthorized);
        }
        if err_lower.contains("credit")
            || err_lower.contains("quota")
            || err_lower.contains("billing")
        {
            return Some(AgentError::OutOfCredits);
        }
        if err_lower.contains("network")
            || err_lower.contains("connection")
            || err_lower.contains("offline")
        {
            return Some(AgentError::NoInternet);
        }
        if err_lower.contains("rate limit") || err_lower.contains("too many requests") {
            return Some(AgentError::RateLimited);
        }
        return Some(AgentError::Other(err.to_string()));
    }

    None
}

/// Find the session log path for a project.
///
/// Uses `CLAUDE_CONFIG_DIR` env var if set, otherwise defaults to `~/.claude`.
pub fn find_session_log(project_path: &Path, session_id: &str) -> Option<PathBuf> {
    let claude_base = std::env::var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".claude"));
    find_session_log_in(project_path, session_id, &claude_base)
}

/// Find the session log path for a project within a specific Claude state directory.
fn find_session_log_in(
    project_path: &Path,
    session_id: &str,
    claude_base: &Path,
) -> Option<PathBuf> {
    // Claude stores logs in <base>/projects/<hash>/<session>.jsonl
    let claude_dir = claude_base.join("projects");

    // Hash the project path to find the right directory
    let project_hash = project_dir_name(project_path);
    let project_dir = claude_dir.join(&project_hash);

    if !project_dir.exists() {
        return None;
    }

    // Look for session file
    let session_file = project_dir.join(format!("{session_id}.jsonl"));
    if session_file.exists() {
        return Some(session_file);
    }

    // Fallback: find most recent .jsonl file
    std::fs::read_dir(&project_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "jsonl").unwrap_or(false))
        .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
        .map(|e| e.path())
}

/// Convert a project path to Claude's directory name format.
///
/// Claude stores project data in `~/.claude/projects/<dir_name>/` where
/// `<dir_name>` is the absolute path with `/` and `.` replaced by `-`.
/// For example: `/Users/foo/.local/bar` → `-Users-foo--local-bar`
///
/// The path is canonicalized first to resolve symlinks (e.g., `/var` → `/private/var`
/// on macOS). This matches Claude Code's internal behavior.
fn project_dir_name(path: &Path) -> String {
    // Canonicalize to resolve symlinks. Claude Code does this internally,
    // so we must match to find the correct project directory.
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical.to_string_lossy().replace(['/', '.'], "-")
}

#[cfg(test)]
#[path = "watcher_tests.rs"]
mod tests;
