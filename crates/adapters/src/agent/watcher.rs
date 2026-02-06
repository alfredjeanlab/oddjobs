// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Background agent watcher using file notifications

use crate::agent::log_entry::{self, AgentLogMessage};
use crate::session::SessionAdapter;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use oj_core::{AgentError, AgentId, AgentState, Event, OwnerId};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};

/// Configuration for the agent watcher
pub(crate) struct WatcherConfig {
    pub agent_id: AgentId,
    /// Session ID for log file lookup (matches --session-id passed to claude)
    pub log_session_id: String,
    /// Session ID for tmux liveness checks (includes oj- prefix)
    pub tmux_session_id: String,
    pub project_path: PathBuf,
    pub process_name: String,
    /// Owner of this agent (job or agent_run)
    pub owner: OwnerId,
}

/// Start watching an agent's session log. Returns a shutdown sender.
pub(crate) fn start_watcher<S: SessionAdapter>(
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
        owner,
    } = config;
    let spawn_time = std::time::Instant::now();

    let wait =
        wait_for_session_log_or_exit(&project_path, &log_session_id, &tmux_session_id, &sessions)
            .await;

    if matches!(&wait, SessionLogWait::SessionDied) {
        tracing::info!(%agent_id, log_session_id, "session ended while waiting for log");
        let _ = event_tx
            .send(Event::AgentGone {
                agent_id,
                owner: owner.clone(),
            })
            .await;
        return;
    }

    let mut log_path = None;
    let mut file_rx = None;
    let _watcher_guard;
    if let SessionLogWait::Found(path) = wait {
        tracing::info!(%agent_id, log_session_id, elapsed_ms = spawn_time.elapsed().as_millis() as u64, "session log found");
        let (tx, rx) = mpsc::channel(32);
        match create_file_watcher(&path, tx) {
            Ok(w) => {
                _watcher_guard = Some(w);
                file_rx = Some(rx);
            }
            Err(e) => {
                tracing::warn!(%agent_id, error = %e, "file watcher failed, using fallback polling");
                _watcher_guard = None;
            }
        }
        log_path = Some(path);
    } else {
        tracing::warn!(%agent_id, log_session_id, "session log not found, using fallback polling");
        _watcher_guard = None;
    }

    watch_loop(WatchLoopParams {
        agent_id,
        tmux_session_id,
        process_name,
        owner,
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
    owner: OwnerId,
    log_path: Option<PathBuf>,
    sessions: S,
    event_tx: mpsc::Sender<Event>,
    shutdown_rx: oneshot::Receiver<()>,
    log_entry_tx: Option<mpsc::Sender<AgentLogMessage>>,
    file_rx: Option<mpsc::Receiver<()>>,
}

async fn watch_loop<S: SessionAdapter>(params: WatchLoopParams<S>) {
    let WatchLoopParams {
        agent_id,
        tmux_session_id,
        process_name,
        owner,
        log_path,
        sessions,
        event_tx,
        mut shutdown_rx,
        log_entry_tx,
        file_rx,
    } = params;

    let mut parser = SessionLogParser::new();
    let mut last_state = log_path
        .as_ref()
        .map(|p| parser.parse(p))
        .unwrap_or(AgentState::Working);
    let mut last_log_offset: u64 = 0;
    let mut file_rx = file_rx;
    let mut poll_count: u64 = 0;

    // Emit non-working state immediately so on_dead fires after reconnect.
    if last_state != AgentState::Working {
        tracing::info!(agent_id = %agent_id, state = ?last_state, "initial state is non-working, emitting immediately");
        emit_state_event(&event_tx, &agent_id, last_state.clone(), &owner).await;
    }

    loop {
        tokio::select! {
            Some(_) = async { match file_rx { Some(ref mut rx) => rx.recv().await, None => std::future::pending().await } } => {
                if let Some(ref log) = log_path {
                    let new_state = parser.parse(log);
                    if new_state != last_state {
                        last_state = new_state.clone();
                        emit_state_event(&event_tx, &agent_id, new_state, &owner).await;
                    }
                    if let Some(ref tx) = log_entry_tx {
                        let (entries, new_offset) = log_entry::parse_entries_from(log, last_log_offset);
                        if !entries.is_empty() {
                            let _ = tx.send((agent_id.clone(), entries)).await;
                        }
                        last_log_offset = new_offset;
                    }
                }
            }

            _ = tokio::time::sleep(crate::env::watcher_poll_ms()) => {
                if let Some(state) = check_liveness(&sessions, &tmux_session_id, &process_name, &agent_id).await {
                    let _ = event_tx.send(Event::from_agent_state(agent_id.clone(), state, owner.clone())).await;
                    break;
                }
                poll_count += 1;
                if log_path.is_none() {
                    if poll_count.is_multiple_of(6) {
                        tracing::debug!(agent_id = %agent_id, tmux_session_id, poll_count, "fallback polling: session alive");
                    } else {
                        tracing::trace!(agent_id = %agent_id, tmux_session_id, poll_count, "fallback polling: session alive");
                    }
                }
            }

            _ = &mut shutdown_rx => {
                tracing::debug!(agent_id = %agent_id, "watcher shutdown requested");
                break;
            }
        }
    }
}

/// Emit a state change event. WaitingForInput is mapped to AgentIdle
/// so the on_idle handler fires without the old timeout delay.
async fn emit_state_event(
    event_tx: &mpsc::Sender<Event>,
    agent_id: &AgentId,
    state: AgentState,
    owner: &OwnerId,
) {
    let event = if state == AgentState::WaitingForInput {
        Event::AgentIdle {
            agent_id: agent_id.clone(),
        }
    } else {
        Event::from_agent_state(agent_id.clone(), state, owner.clone())
    };
    let _ = event_tx.send(event).await;
}

/// Check whether an agent's session and process are still alive.
/// Returns `Some(state)` if the agent has terminated, or `None` if still running.
async fn check_liveness<S: SessionAdapter>(
    sessions: &S,
    session_id: &str,
    process_name: &str,
    agent_id: &AgentId,
) -> Option<AgentState> {
    match sessions.is_alive(session_id).await {
        Ok(false) | Err(_) => {
            let exit_code = sessions.get_exit_code(session_id).await.ok().flatten();
            tracing::info!(%agent_id, session_id, ?exit_code, "tmux session gone");
            Some(AgentState::SessionGone)
        }
        Ok(true) => match sessions.is_process_running(session_id, process_name).await {
            Ok(false) => {
                let exit_code = sessions.get_exit_code(session_id).await.ok().flatten();
                tracing::info!(%agent_id, session_id, process_name, ?exit_code, "agent process exited");
                Some(AgentState::Exited { exit_code })
            }
            Ok(true) => None,
            Err(e) => {
                tracing::warn!(%agent_id, session_id, error = %e, "failed to check agent process");
                None
            }
        },
    }
}

enum SessionLogWait {
    Found(PathBuf),
    SessionDied,
    Timeout,
}

/// Check pane output for trust prompt and auto-accept if found.
async fn check_and_accept_trust_prompt<S: SessionAdapter>(
    sessions: &S,
    tmux_session_id: &str,
) -> bool {
    use super::claude::{poll_for_prompt, PromptCheck, PromptResult};
    matches!(
        poll_for_prompt(
            sessions,
            tmux_session_id,
            1,
            &PromptCheck {
                detect: &["Do you trust the files in this folder?", "Do you trust"],
                match_any: true,
                response: Some("y"),
                check_errors: false,
            }
        )
        .await,
        Ok(PromptResult::Handled)
    )
}

/// Wait for session log to be created (up to 30 seconds), checking session
/// liveness and trust prompts in parallel.
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
        if let Ok(false) = sessions.is_alive(tmux_session_id).await {
            tracing::debug!(
                log_session_id,
                tmux_session_id,
                iteration = i,
                "session died while waiting for log"
            );
            return SessionLogWait::SessionDied;
        }
        if i < 5 {
            check_and_accept_trust_prompt(sessions, tmux_session_id).await;
        }
        tokio::time::sleep(crate::env::session_poll_ms()).await;
    }
    tracing::warn!(log_session_id, project_path = %project_path.display(), "gave up waiting for session log after 30s");
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
            return AgentState::Working;
        };
        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if file_len < self.last_offset {
            self.last_offset = 0;
            self.last_line.clear();
        }
        if file_len == self.last_offset {
            return parse_state_from_line(&self.last_line);
        }
        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(self.last_offset)).is_err() {
            return parse_state_from_line(&self.last_line);
        }
        let mut current_offset = self.last_offset;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let complete = line.ends_with('\n');
                    if complete {
                        current_offset += n as u64;
                    }
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        if complete {
                            continue;
                        } else {
                            break;
                        }
                    }
                    self.last_line.clear();
                    self.last_line.push_str(trimmed);
                    if !complete {
                        break;
                    }
                }
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
    let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
        return AgentState::Working;
    };
    if let Some(failure) = detect_error(&json) {
        return AgentState::Failed(failure);
    }
    if json.get("type").and_then(|v| v.as_str()) != Some("assistant") {
        return AgentState::Working;
    }
    let msg = json.get("message");
    let stop_reason = msg.and_then(|m| m.get("stop_reason"));
    if matches!(stop_reason, Some(sr) if !sr.is_null()) {
        tracing::warn!(stop_reason = ?stop_reason, "unexpected non-null stop_reason, assuming working");
        return AgentState::Working;
    }
    // Both tool_use and thinking blocks indicate the agent is actively working
    let has_active = msg
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .is_some_and(|arr| {
            arr.iter().any(|item| {
                matches!(
                    item.get("type").and_then(|v| v.as_str()),
                    Some("tool_use" | "thinking")
                )
            })
        });
    if has_active {
        AgentState::Working
    } else {
        AgentState::WaitingForInput
    }
}

fn detect_error(json: &serde_json::Value) -> Option<AgentError> {
    let err = json.get("error").and_then(|v| v.as_str()).or_else(|| {
        json.get("message")
            .and_then(|m| m.get("error"))
            .and_then(|v| v.as_str())
    })?;
    let lower = err.to_lowercase();
    let has = |ps: &[&str]| ps.iter().any(|p| lower.contains(p));
    Some(if has(&["unauthorized", "invalid api key"]) {
        AgentError::Unauthorized
    } else if has(&["credit", "quota", "billing"]) {
        AgentError::OutOfCredits
    } else if has(&["network", "connection", "offline"]) {
        AgentError::NoInternet
    } else if has(&["rate limit", "too many requests"]) {
        AgentError::RateLimited
    } else {
        AgentError::Other(err.to_string())
    })
}

/// Extract the last assistant text message from a Claude JSONL session log.
///
/// Reads the tail of the file, iterates in reverse to find the last `"type": "assistant"`
/// line, and concatenates all `{"type":"text","text":"..."}` content blocks.
/// Returns `None` if no assistant message is found.
pub fn extract_last_assistant_text(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

    // Search last ~50 lines in reverse for the last assistant message
    for line in lines.iter().rev().take(50) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let json: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if json.get("type").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        let content = json
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())?;

        let text: String = content
            .iter()
            .filter(|item| item.get("type").and_then(|v| v.as_str()) == Some("text"))
            .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n");

        if !text.is_empty() {
            return Some(text);
        }
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
    let project_dir = claude_base
        .join("projects")
        .join(project_dir_name(project_path));
    let session_file = project_dir.join(format!("{session_id}.jsonl"));
    if session_file.exists() {
        return Some(session_file);
    }
    // Fallback: find most recent .jsonl file
    std::fs::read_dir(&project_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
        .map(|e| e.path())
}

/// Convert a project path to Claude's directory name format (replace `/` and `.` with `-`).
fn project_dir_name(path: &Path) -> String {
    // Canonicalize to resolve symlinks. Claude Code does this internally,
    // so we must match to find the correct project directory.
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical.to_string_lossy().replace(['/', '.'], "-")
}

#[cfg(test)]
#[path = "watcher_tests/mod.rs"]
mod tests;
