// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! WebSocket event bridge — subscribes to coop's state+messages stream and
//! translates coop events into oddjobs `Event` values.

use super::http;
use crate::agent::log_entry::{self, AgentLogMessage};
use futures_util::StreamExt;
use oj_core::{AgentId, Event, OwnerId};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

/// Background task that subscribes to coop's WebSocket state+messages stream and emits events.
pub(super) async fn event_bridge(
    socket_path: PathBuf,
    agent_id: AgentId,
    owner: OwnerId,
    event_tx: mpsc::Sender<Event>,
    shutdown_rx: oneshot::Receiver<()>,
    log_entry_tx: Option<mpsc::Sender<AgentLogMessage>>,
) {
    let mut shutdown_rx = shutdown_rx;

    // Connect WebSocket over Unix socket
    let ws_stream = match connect_ws(&socket_path).await {
        Some(s) => {
            tracing::info!(%agent_id, "ws bridge connected");
            s
        }
        None => {
            tracing::warn!(%agent_id, "ws bridge: connection failed, emitting AgentGone");
            let _ = event_tx
                .send(Event::AgentGone {
                    id: agent_id.clone(),
                    owner: owner.clone(),
                    exit_code: None,
                })
                .await;
            return;
        }
    };

    let (_, mut read) = ws_stream.split();

    // Catch up: state transitions may have occurred before the WS subscription
    // started (broadcast channels only deliver messages sent after subscribe).
    // Poll the current state via HTTP and emit the initial event.
    //
    // Bounded by a 3-second timeout so the WS event loop can start even if
    // coop's HTTP handler blocks (e.g., waiting for the session to become ready
    // while the agent has already exited).
    match tokio::time::timeout(
        Duration::from_secs(3),
        poll_initial_state(&socket_path, &agent_id, &owner),
    )
    .await
    {
        Ok(Some(event)) => {
            tracing::info!(%agent_id, ?event, "ws bridge: initial state event");
            let _ = event_tx.send(event).await;
        }
        _ => {}
    }

    tracing::info!(%agent_id, "ws bridge: entering event loop");

    // Track last user timestamp for log entry extraction (turn duration)
    let mut last_user_timestamp: Option<String> = None;

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match parse_ws_event(&text, &agent_id, &owner) {
                            WsParseResult::Event(event) => {
                                tracing::info!(%agent_id, ?event, "ws bridge: emitting event");
                                let _ = event_tx.send(*event).await;
                            }
                            WsParseResult::None => {}
                        }

                        // Extract log entries from message:raw events
                        if let Some(ref tx) = log_entry_tx {
                            if let Some(entries) = extract_log_entries_from_ws(&text, &mut last_user_timestamp) {
                                if !entries.is_empty() {
                                    let _ = tx.send((agent_id.clone(), entries)).await;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        tracing::info!(%agent_id, ?frame, "ws bridge: received close frame");
                        let _ = event_tx.send(Event::AgentGone {
                            id: agent_id.clone(),
                            owner: owner.clone(),
                            exit_code: None,
                        }).await;
                        break;
                    }
                    None => {
                        tracing::info!(%agent_id, "ws bridge: stream ended");
                        let _ = event_tx.send(Event::AgentGone {
                            id: agent_id.clone(),
                            owner: owner.clone(),
                            exit_code: None,
                        }).await;
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::warn!(%agent_id, %e, "ws bridge: error");
                        let _ = event_tx.send(Event::AgentGone {
                            id: agent_id.clone(),
                            owner: owner.clone(),
                            exit_code: None,
                        }).await;
                        break;
                    }
                    _ => {} // Ping/Pong/Binary — ignore
                }
            }
            _ = &mut shutdown_rx => {
                break;
            }
        }
    }
}

/// Connect a WebSocket over a Unix socket to coop's state subscription endpoint.
async fn connect_ws(
    socket_path: &Path,
) -> Option<tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>> {
    // Retry a few times while coop starts up
    let stream = {
        let mut stream = None;
        for i in 0..10 {
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            match tokio::net::UnixStream::connect(socket_path).await {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                _ => {}
            }
        }
        if stream.is_none() {
            tracing::warn!(path = %socket_path.display(), "ws connect: failed after 10 attempts");
        }
        stream?
    };

    let uri = "ws://localhost/ws?subscribe=state,messages";
    match tokio_tungstenite::client_async(uri, stream).await {
        Ok((ws, _)) => Some(ws),
        Err(e) => {
            tracing::warn!(
                path = %socket_path.display(),
                error = %e,
                "ws connect: WebSocket handshake failed"
            );
            None
        }
    }
}

/// Poll coop's HTTP endpoint for the current state and return an event if
/// it's already actionable. This closes the race where state transitions
/// occur before the WebSocket subscription is established.
async fn poll_initial_state(
    socket_path: &Path,
    agent_id: &AgentId,
    owner: &OwnerId,
) -> Option<Event> {
    tracing::info!(%agent_id, "poll_initial_state: sending HTTP GET /api/v1/agent");
    let body = match http::get(socket_path, "/api/v1/agent").await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(%agent_id, %e, "poll_initial_state: HTTP request failed");
            return None;
        }
    };
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    map_initial_state(&json, agent_id, owner)
}

/// Result of parsing a WebSocket frame.
pub(crate) enum WsParseResult {
    /// An event to emit.
    Event(Box<Event>),
    /// Nothing to emit.
    None,
}

/// Parse a coop WebSocket JSON frame into an oddjobs Event.
pub(crate) fn parse_ws_event(text: &str, agent_id: &AgentId, owner: &OwnerId) -> WsParseResult {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return WsParseResult::None;
    };
    let event_type = json.get("event").and_then(|v| v.as_str()).unwrap_or("");

    match event_type {
        "transition" => {
            let Some(next) = json.get("next").and_then(|v| v.as_str()) else {
                return WsParseResult::None;
            };
            match next {
                "idle" => WsParseResult::Event(Box::new(Event::AgentIdle { id: agent_id.clone() })),
                "working" => WsParseResult::Event(Box::new(Event::AgentWorking {
                    id: agent_id.clone(),
                    owner: owner.clone(),
                })),
                "prompt" => {
                    let prompt_type = json
                        .get("prompt")
                        .and_then(|p| p.get("type"))
                        .and_then(|v| v.as_str())
                        .map(map_prompt_type)
                        .unwrap_or(oj_core::PromptType::Permission);

                    let questions = if prompt_type == oj_core::PromptType::Question {
                        parse_questions(&json)
                    } else {
                        None
                    };

                    let last_message = extract_last_message(&json, &prompt_type);

                    WsParseResult::Event(Box::new(Event::AgentPrompt {
                        id: agent_id.clone(),
                        prompt_type,
                        questions,
                        last_message,
                    }))
                }
                "error" => {
                    let category = json.get("error_category").and_then(|v| v.as_str());
                    let detail = json
                        .get("error_detail")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    let error = match category {
                        Some("Unauthorized") => oj_core::AgentError::Unauthorized,
                        Some("OutOfCredits") => oj_core::AgentError::OutOfCredits,
                        _ => oj_core::AgentError::Other(detail.to_string()),
                    };
                    WsParseResult::Event(Box::new(Event::AgentFailed {
                        id: agent_id.clone(),
                        error,
                        owner: owner.clone(),
                    }))
                }
                "exited" => WsParseResult::Event(Box::new(Event::AgentGone {
                    id: agent_id.clone(),
                    owner: owner.clone(),
                    exit_code: None,
                })),
                _ => WsParseResult::None,
            }
        }
        "exit" => {
            // Emit AgentGone immediately on "exit" events. Coop may not send a
            // subsequent "transition: exited" when the child exits before the
            // session is fully ready (e.g., print-mode agents).
            let code = json.get("code").and_then(|v| v.as_i64()).map(|c| c as i32);
            WsParseResult::Event(Box::new(Event::AgentGone {
                id: agent_id.clone(),
                owner: owner.clone(),
                exit_code: code,
            }))
        }
        "stop:outcome" => {
            let outcome_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match outcome_type {
                "blocked" => {
                    WsParseResult::Event(Box::new(Event::AgentStopBlocked { id: agent_id.clone() }))
                }
                "allowed" => {
                    WsParseResult::Event(Box::new(Event::AgentStopAllowed { id: agent_id.clone() }))
                }
                _ => WsParseResult::None,
            }
        }
        _ => WsParseResult::None,
    }
}

pub(crate) fn map_prompt_type(s: &str) -> oj_core::PromptType {
    match s {
        "permission" => oj_core::PromptType::Permission,
        "plan" => oj_core::PromptType::PlanApproval,
        "question" => oj_core::PromptType::Question,
        _ => oj_core::PromptType::Permission,
    }
}

/// Extract the assistant context string from a coop prompt event.
///
/// For plan prompts, coop stores the plan content in `prompt.input` (a JSON
/// string containing an ExitPlanMode tool input with a `plan` field). For
/// other prompts, fall back to `last_message`.
pub(crate) fn extract_last_message(
    json: &serde_json::Value,
    prompt_type: &oj_core::PromptType,
) -> Option<String> {
    if *prompt_type == oj_core::PromptType::PlanApproval {
        // Try prompt.input first — contains the ExitPlanMode tool input JSON
        if let Some(input_str) =
            json.get("prompt").and_then(|p| p.get("input")).and_then(|v| v.as_str())
        {
            if let Ok(input_json) = serde_json::from_str::<serde_json::Value>(input_str) {
                if let Some(plan) = input_json.get("plan").and_then(|v| v.as_str()) {
                    return Some(plan.to_string());
                }
            }
        }
    }
    // Default: use last_message
    json.get("last_message")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

pub(crate) fn parse_questions(json: &serde_json::Value) -> Option<oj_core::QuestionData> {
    let prompt = json.get("prompt")?;
    let questions = prompt.get("questions")?.as_array()?;
    let entries: Vec<oj_core::QuestionEntry> = questions
        .iter()
        .filter_map(|q| {
            let question = q.get("question")?.as_str()?.to_string();
            let options = q
                .get("options")
                .and_then(|o| o.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|opt| {
                            // Coop sends options as plain strings; handle both
                            // string and {label, description} object formats.
                            if let Some(label) = opt.as_str() {
                                Some(oj_core::QuestionOption {
                                    label: label.to_string(),
                                    description: None,
                                })
                            } else {
                                Some(oj_core::QuestionOption {
                                    label: opt.get("label")?.as_str()?.to_string(),
                                    description: opt
                                        .get("description")
                                        .and_then(|d| d.as_str())
                                        .map(|s| s.to_string()),
                                })
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(oj_core::QuestionEntry {
                question,
                header: q.get("header").and_then(|h| h.as_str()).map(|s| s.to_string()),
                multi_select: q.get("multiSelect").and_then(|m| m.as_bool()).unwrap_or(false),
                options,
            })
        })
        .collect();

    if entries.is_empty() {
        None
    } else {
        Some(oj_core::QuestionData { questions: entries })
    }
}

/// Map a coop `/api/v1/agent` JSON response to an initial-state `Event`.
///
/// Returns `Some` only for actionable states (idle, exited, error, prompt);
/// "starting" and "working" return `None` because the WS stream will deliver
/// subsequent transitions.
pub(crate) fn map_initial_state(
    json: &serde_json::Value,
    agent_id: &AgentId,
    owner: &OwnerId,
) -> Option<Event> {
    let state = json.get("state").and_then(|v| v.as_str())?;
    match state {
        "idle" => Some(Event::AgentIdle { id: agent_id.clone() }),
        "exited" => {
            Some(Event::AgentGone { id: agent_id.clone(), owner: owner.clone(), exit_code: None })
        }
        "error" => {
            let category = json.get("error_category").and_then(|v| v.as_str());
            let detail =
                json.get("error_detail").and_then(|v| v.as_str()).unwrap_or("unknown error");
            let error = match category {
                Some("Unauthorized") => oj_core::AgentError::Unauthorized,
                Some("OutOfCredits") => oj_core::AgentError::OutOfCredits,
                _ => oj_core::AgentError::Other(detail.to_string()),
            };
            Some(Event::AgentFailed { id: agent_id.clone(), error, owner: owner.clone() })
        }
        "prompt" => {
            let prompt_type = json
                .get("prompt")
                .and_then(|p| p.get("type"))
                .and_then(|v| v.as_str())
                .map(map_prompt_type)
                .unwrap_or(oj_core::PromptType::Permission);
            let questions = if prompt_type == oj_core::PromptType::Question {
                parse_questions(json)
            } else {
                None
            };
            let last_message = extract_last_message(json, &prompt_type);
            Some(Event::AgentPrompt { id: agent_id.clone(), prompt_type, questions, last_message })
        }
        // "starting" | "working" — not actionable, wait for WS events
        _ => None,
    }
}

/// Extract log entries from a `message:raw` WebSocket event.
///
/// Coop pushes `{ "event": "message:raw", "data": {...} }` frames where
/// `data` is the full JSONL record. We pass it to `extract_entries()` which
/// handles the same JSON shape that `parse_entries_from()` used to read from files.
pub(crate) fn extract_log_entries_from_ws(
    text: &str,
    last_user_timestamp: &mut Option<String>,
) -> Option<Vec<log_entry::AgentLogEntry>> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    if json.get("event").and_then(|v| v.as_str()) != Some("message:raw") {
        return None;
    }
    let data = json.get("data")?;
    let mut entries = Vec::new();
    log_entry::extract_entries(data, &mut entries, last_user_timestamp);
    Some(entries)
}
