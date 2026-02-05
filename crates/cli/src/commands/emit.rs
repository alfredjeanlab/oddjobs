// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Emit commands for agent-to-daemon signaling

use anyhow::Result;
use clap::{Args, Subcommand};
use oj_core::{AgentId, AgentSignalKind, Event, PromptType};
use serde::Deserialize;
use std::io::Read;

use crate::client::DaemonClient;
use crate::output::OutputFormat;

#[derive(Args)]
pub struct EmitArgs {
    #[command(subcommand)]
    pub command: EmitCommand,
}

#[derive(Subcommand)]
// Variant names match CLI subcommand names (agent:signal, agent:idle, agent:prompt)
#[allow(clippy::enum_variant_names)]
pub enum EmitCommand {
    /// Signal agent completion to the daemon
    #[command(name = "agent:signal")]
    AgentDone {
        /// Agent ID (required - no longer read from environment)
        #[arg(long = "agent")]
        agent_id: String,

        /// Signal payload: "complete", "escalate", "continue", or JSON {"kind": "complete"}
        /// If omitted, reads from stdin
        #[arg(value_name = "PAYLOAD")]
        payload: Option<String>,
    },

    /// Report agent idle (from Notification hook)
    #[command(name = "agent:idle")]
    AgentIdle {
        #[arg(long = "agent")]
        agent_id: String,
    },

    /// Report agent prompt (from Notification hook)
    #[command(name = "agent:prompt")]
    AgentPrompt {
        #[arg(long = "agent")]
        agent_id: String,
        #[arg(long = "type", default_value = "other")]
        prompt_type: String,
    },
}

/// JSON payload structure for agent:signal command
#[derive(Debug, Deserialize)]
struct AgentDonePayload {
    kind: AgentSignalKind,
    #[serde(default)]
    message: Option<String>,
}

/// Parse agent:signal payload from a string.
///
/// Accepts:
/// - Plain strings: "complete", "escalate", "continue"
/// - JSON objects: {"kind": "complete"}, {"kind": "escalate", "message": "..."}
/// - Relaxed forms: {kind: complete} (treated as plain-string fallback)
fn parse_signal_payload(input: &str) -> Result<AgentDonePayload> {
    let trimmed = input.trim();

    // Try plain-string shortcuts first (most common from agents)
    match trimmed {
        "complete" => {
            return Ok(AgentDonePayload {
                kind: AgentSignalKind::Complete,
                message: None,
            });
        }
        "escalate" => {
            return Ok(AgentDonePayload {
                kind: AgentSignalKind::Escalate,
                message: None,
            });
        }
        "continue" => {
            return Ok(AgentDonePayload {
                kind: AgentSignalKind::Continue,
                message: None,
            });
        }
        _ => {}
    }

    // Try JSON parsing
    serde_json::from_str(trimmed).map_err(|e| {
        anyhow::anyhow!(
            "invalid signal payload: {}. Use: complete, escalate, continue, or JSON {{\"kind\": \"complete\"}}",
            e
        )
    })
}

/// Parse a prompt type string to PromptType enum
fn parse_prompt_type(s: &str) -> PromptType {
    match s {
        "permission" => PromptType::Permission,
        "idle" => PromptType::Idle,
        "plan_approval" => PromptType::PlanApproval,
        "question" => PromptType::Question,
        _ => PromptType::Other,
    }
}

#[cfg(test)]
#[path = "emit_tests.rs"]
mod tests;

pub async fn handle(
    command: EmitCommand,
    client: &DaemonClient,
    _format: OutputFormat,
) -> Result<()> {
    match command {
        EmitCommand::AgentDone { agent_id, payload } => {
            // Read JSON from arg or stdin
            let json_str = match payload {
                Some(s) => s,
                None => {
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                }
            };

            let payload: AgentDonePayload = parse_signal_payload(&json_str)?;

            let event = Event::AgentSignal {
                agent_id: AgentId::new(agent_id),
                kind: payload.kind,
                message: payload.message,
            };

            client.emit_event(event).await?;
            Ok(())
        }
        EmitCommand::AgentIdle { agent_id } => {
            let event = Event::AgentIdle {
                agent_id: AgentId::new(agent_id),
            };
            client.emit_event(event).await?;
            Ok(())
        }
        EmitCommand::AgentPrompt {
            agent_id,
            prompt_type,
        } => {
            let event = Event::AgentPrompt {
                agent_id: AgentId::new(agent_id),
                prompt_type: parse_prompt_type(&prompt_type),
                question_data: None,
            };
            client.emit_event(event).await?;
            Ok(())
        }
    }
}
