// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cross-entity ID resolution for convenience commands.
//!
//! Resolves an ID across pipelines, agents, and sessions by exact or prefix match,
//! then dispatches to the appropriate typed subcommand.

use anyhow::Result;

use crate::client::DaemonClient;
use crate::output::OutputFormat;

/// The kind of entity matched during resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityKind {
    Pipeline,
    Agent,
    Session,
}

impl EntityKind {
    fn as_str(&self) -> &'static str {
        match self {
            EntityKind::Pipeline => "pipeline",
            EntityKind::Agent => "agent",
            EntityKind::Session => "session",
        }
    }
}

/// A resolved entity match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityMatch {
    pub kind: EntityKind,
    pub id: String,
    /// Human-readable label (e.g. pipeline name, agent step name)
    pub label: Option<String>,
}

/// Resolve an ID across all entity types.
///
/// Returns all matches. Exact matches take priority over prefix matches:
/// if any exact match is found, prefix matches are discarded.
pub async fn resolve_entity(client: &DaemonClient, query: &str) -> Result<Vec<EntityMatch>> {
    let pipelines = client.list_pipelines().await?;
    let agents = client.list_agents(None, None).await?;
    let sessions = client.list_sessions().await?;
    Ok(resolve_from_lists(query, &pipelines, &agents, &sessions))
}

/// Pure function for entity resolution — testable without async client.
pub fn resolve_from_lists(
    query: &str,
    pipelines: &[oj_daemon::PipelineSummary],
    agents: &[oj_daemon::AgentSummary],
    sessions: &[oj_daemon::SessionSummary],
) -> Vec<EntityMatch> {
    let mut exact = Vec::new();
    let mut prefix = Vec::new();

    for p in pipelines {
        if p.id == query {
            exact.push(EntityMatch {
                kind: EntityKind::Pipeline,
                id: p.id.clone(),
                label: Some(p.name.clone()),
            });
        } else if p.id.starts_with(query) {
            prefix.push(EntityMatch {
                kind: EntityKind::Pipeline,
                id: p.id.clone(),
                label: Some(p.name.clone()),
            });
        }
    }

    for a in agents {
        if a.agent_id == query {
            exact.push(EntityMatch {
                kind: EntityKind::Agent,
                id: a.agent_id.clone(),
                label: a.agent_name.clone(),
            });
        } else if a.agent_id.starts_with(query) {
            prefix.push(EntityMatch {
                kind: EntityKind::Agent,
                id: a.agent_id.clone(),
                label: a.agent_name.clone(),
            });
        }
    }

    for s in sessions {
        if s.id == query {
            exact.push(EntityMatch {
                kind: EntityKind::Session,
                id: s.id.clone(),
                label: None,
            });
        } else if s.id.starts_with(query) {
            prefix.push(EntityMatch {
                kind: EntityKind::Session,
                id: s.id.clone(),
                label: None,
            });
        }
    }

    if exact.is_empty() {
        prefix
    } else {
        exact
    }
}

/// Resolve a single entity or exit with an error for ambiguous/no-match cases.
async fn resolve_one(
    client: &DaemonClient,
    query: &str,
    command_name: &str,
) -> Result<EntityMatch> {
    let matches = resolve_entity(client, query).await?;
    if matches.is_empty() {
        eprintln!("no entity found matching '{}'", query);
        std::process::exit(1);
    } else if matches.len() > 1 {
        print_ambiguous(query, command_name, &matches);
        std::process::exit(1);
    } else {
        Ok(matches
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no entity found matching '{}'", query))?)
    }
}

/// Print ambiguous matches to stderr.
fn print_ambiguous(query: &str, command_name: &str, matches: &[EntityMatch]) {
    eprintln!("Ambiguous ID '{}' — matches multiple entities:\n", query);
    for m in matches {
        let label = m.label.as_deref().unwrap_or("");
        eprintln!(
            "  oj {} {} {}  {}",
            m.kind.as_str(),
            command_name,
            m.id,
            label
        );
    }
}

// ── Convenience command handlers ──────────────────────────────────────────

pub async fn handle_peek(client: &DaemonClient, id: &str, format: OutputFormat) -> Result<()> {
    let entity = resolve_one(client, id, "peek").await?;
    match entity.kind {
        EntityKind::Pipeline => {
            super::pipeline::handle(
                super::pipeline::PipelineCommand::Peek { id: entity.id },
                client,
                None,
                format,
            )
            .await
        }
        EntityKind::Agent => {
            super::agent::handle(
                super::agent::AgentCommand::Peek { id: entity.id },
                client,
                "",
                None,
                format,
            )
            .await
        }
        EntityKind::Session => {
            super::session::handle(
                super::session::SessionCommand::Peek { id: entity.id },
                client,
                "",
                None,
                format,
            )
            .await
        }
    }
}

pub async fn handle_attach(client: &DaemonClient, id: &str) -> Result<()> {
    let entity = resolve_one(client, id, "attach").await?;
    match entity.kind {
        EntityKind::Pipeline => {
            super::pipeline::handle(
                super::pipeline::PipelineCommand::Attach { id: entity.id },
                client,
                None,
                OutputFormat::Text,
            )
            .await
        }
        EntityKind::Agent => {
            super::agent::handle(
                super::agent::AgentCommand::Attach { id: entity.id },
                client,
                "",
                None,
                OutputFormat::Text,
            )
            .await
        }
        EntityKind::Session => super::session::attach(&entity.id),
    }
}

pub async fn handle_logs(
    client: &DaemonClient,
    id: &str,
    follow: bool,
    limit: usize,
    step: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    let entity = resolve_one(client, id, "logs").await?;
    match entity.kind {
        EntityKind::Pipeline => {
            super::pipeline::handle(
                super::pipeline::PipelineCommand::Logs {
                    id: entity.id,
                    follow,
                    limit,
                },
                client,
                None,
                format,
            )
            .await
        }
        EntityKind::Agent => {
            super::agent::handle(
                super::agent::AgentCommand::Logs {
                    id: entity.id,
                    step: step.map(String::from),
                    follow,
                    limit,
                },
                client,
                "",
                None,
                format,
            )
            .await
        }
        EntityKind::Session => {
            eprintln!(
                "logs are not available for sessions — use 'oj peek {}' instead",
                entity.id
            );
            std::process::exit(1);
        }
    }
}

pub async fn handle_show(
    client: &DaemonClient,
    id: &str,
    verbose: bool,
    format: OutputFormat,
) -> Result<()> {
    let entity = resolve_one(client, id, "show").await?;
    match entity.kind {
        EntityKind::Pipeline => {
            super::pipeline::handle(
                super::pipeline::PipelineCommand::Show {
                    id: entity.id,
                    verbose,
                },
                client,
                None,
                format,
            )
            .await
        }
        EntityKind::Agent => {
            super::agent::handle(
                super::agent::AgentCommand::Show { id: entity.id },
                client,
                "",
                None,
                format,
            )
            .await
        }
        EntityKind::Session => {
            super::session::handle(
                super::session::SessionCommand::Show { id: entity.id },
                client,
                "",
                None,
                format,
            )
            .await
        }
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
