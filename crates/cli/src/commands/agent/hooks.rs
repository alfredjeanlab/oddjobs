// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Hook handlers for Claude Code integration (stop, pretooluse, notify).

use std::io::{self, Read, Write};

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use oj_core::{AgentId, Event, PromptType, QuestionData};

use crate::client::DaemonClient;

use super::utils::{append_agent_log, get_state_dir, prompt_type_for_tool};

/// Input from Claude Code PreToolUse hook (subset of fields we care about)
#[derive(Default, Deserialize)]
pub(super) struct PreToolUseInput {
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: Option<serde_json::Value>,
    #[serde(default)]
    transcript_path: Option<String>,
}

/// Input from Claude Code Stop hook (subset of fields we care about)
#[derive(Default, Deserialize)]
struct StopHookInput {
    #[serde(default)]
    stop_hook_active: bool,
}

/// Output to Claude Code Stop hook
#[derive(Serialize)]
struct StopHookOutput {
    decision: String,
    reason: String,
}

/// Input from Claude Code Notification hook (subset of fields we care about)
#[derive(Debug, Default, Deserialize)]
pub(super) struct NotificationHookInput {
    #[serde(default)]
    pub notification_type: String,
}

/// Read and deserialize JSON from stdin, returning `T::default()` on parse failure.
fn read_hook_input<T: DeserializeOwned + Default>() -> Result<T> {
    let mut input_json = String::new();
    io::stdin().read_to_string(&mut input_json)?;
    Ok(serde_json::from_str(&input_json).unwrap_or_default())
}

pub(super) async fn handle_pretooluse(agent_id: &str, client: &DaemonClient) -> Result<()> {
    let input: PreToolUseInput = read_hook_input()?;

    let Some(prompt_type) = prompt_type_for_tool(input.tool_name.as_deref()) else {
        return Ok(());
    };

    // Extract question data from AskUserQuestion tool_input
    let question_data = if prompt_type == PromptType::Question {
        input
            .tool_input
            .as_ref()
            .and_then(|v| serde_json::from_value::<QuestionData>(v.clone()).ok())
    } else {
        None
    };

    // Extract context for decision display.
    // For PlanApproval, extract plan content from tool_input.plan (where Claude sends it).
    // For other prompts, fall back to the last assistant message from the transcript.
    let assistant_context = if prompt_type == PromptType::PlanApproval {
        input
            .tool_input
            .as_ref()
            .and_then(|v| v.get("plan"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                input
                    .transcript_path
                    .as_deref()
                    .map(std::path::Path::new)
                    .and_then(extract_last_assistant_text)
            })
    } else {
        input
            .transcript_path
            .as_deref()
            .map(std::path::Path::new)
            .and_then(extract_last_assistant_text)
    };

    let event = Event::AgentPrompt {
        agent_id: AgentId::new(agent_id),
        prompt_type,
        question_data,
        assistant_context,
    };
    client.emit_event(event).await?;

    Ok(())
}

pub(super) async fn handle_notify(agent_id: &str, client: &DaemonClient) -> Result<()> {
    let input: NotificationHookInput = read_hook_input()?;

    match input.notification_type.as_str() {
        "idle_prompt" => {
            let event = Event::AgentIdle {
                agent_id: AgentId::new(agent_id),
            };
            client.emit_event(event).await?;
        }
        "permission_prompt" => {
            let event = Event::AgentPrompt {
                agent_id: AgentId::new(agent_id),
                prompt_type: PromptType::Permission,
                question_data: None,
                assistant_context: None,
            };
            client.emit_event(event).await?;
        }
        _ => {
            // Ignore other notification types
        }
    }

    Ok(())
}

pub(super) async fn handle_stop(agent_id: &str, client: &DaemonClient) -> Result<()> {
    let input: StopHookInput = read_hook_input()?;

    append_agent_log(
        agent_id,
        &format!("invoked, stop_hook_active={}", input.stop_hook_active),
    );

    // CRITICAL: Prevent infinite loops
    // If stop_hook_active is true, we're already in a stop hook chain - allow exit
    if input.stop_hook_active {
        append_agent_log(agent_id, "allowing exit, stop_hook_active=true");
        std::process::exit(0);
    }

    // Read on_stop config from agent state dir
    let on_stop = read_on_stop_config(agent_id);

    // Query daemon: has this agent signaled completion?
    let response = client.query_agent_signal(agent_id).await?;

    if response.signaled {
        // Agent has called `oj emit agent:signal` - allow stop
        append_agent_log(agent_id, "allowing exit, signaled=true");
        std::process::exit(0);
    }

    append_agent_log(
        agent_id,
        &format!("blocking exit, on_stop={}, signaled=false", on_stop),
    );

    match on_stop.as_str() {
        "idle" => {
            // Emit idle event, then block
            let event = Event::AgentIdle {
                agent_id: AgentId::new(agent_id),
            };
            let _ = client.emit_event(event).await;
            block_exit("Stop hook: on_idle handler invoked. Waiting for further instructions.");
        }
        "escalate" => {
            // Emit stop event for escalation, then block
            let event = Event::AgentStop {
                agent_id: AgentId::new(agent_id),
            };
            let _ = client.emit_event(event).await;
            block_exit("A human has been notified. Wait for instructions.");
        }
        "ask" => {
            // Read topic from config, then block with AskUserQuestion instruction
            let topic = read_config_message(agent_id)
                .unwrap_or_else(|| "What should I do next?".to_string());
            block_exit(&format!(
                "Use the AskUserQuestion tool to ask the user: '{}'\n\n\
                 Do not proceed without asking this question first.",
                topic
            ));
        }
        _ => {
            // "signal" (default) — current behavior
            block_exit(&format!(
                "You must signal before stopping. Run one of:\n\
                 oj emit agent:signal --agent {} complete  — task is done\n\
                 oj emit agent:signal --agent {} escalate  — need human help\n\
                 oj emit agent:signal --agent {} continue  — still working on the task",
                agent_id, agent_id, agent_id
            ));
        }
    }
}

fn read_on_stop_config(agent_id: &str) -> String {
    let state_dir = get_state_dir();
    let config_path = state_dir.join("agents").join(agent_id).join("config.json");
    std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("on_stop")?.as_str().map(String::from))
        .unwrap_or_else(|| "signal".to_string())
}

fn read_config_message(agent_id: &str) -> Option<String> {
    let state_dir = get_state_dir();
    let config_path = state_dir.join("agents").join(agent_id).join("config.json");
    std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("message")?.as_str().map(String::from))
}

/// Extract the last assistant text message from a Claude JSONL session log.
///
/// Reads the tail of the file, iterates in reverse to find the last `"type": "assistant"`
/// line, and concatenates all `{"type":"text","text":"..."}` content blocks.
fn extract_last_assistant_text(path: &std::path::Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let reader = io::BufReader::new(file);
    let lines: Vec<String> = io::BufRead::lines(reader).map_while(Result::ok).collect();

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

fn block_exit(reason: &str) -> ! {
    let output = StopHookOutput {
        decision: "block".to_string(),
        reason: reason.to_string(),
    };
    let output_json = serde_json::to_string(&output).unwrap_or_default();
    let _ = io::stdout().write_all(output_json.as_bytes());
    let _ = io::stdout().flush();
    std::process::exit(0);
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
