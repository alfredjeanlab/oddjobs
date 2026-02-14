// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Cross-entity ID resolution for convenience commands.
//!
//! Resolves an ID across jobs and agents by exact or prefix match,
//! then dispatches to the appropriate typed subcommand.

use anyhow::Result;

use crate::client::DaemonClient;
use crate::output::OutputFormat;

/// The kind of entity matched during resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityKind {
    Job,
    Agent,
}

impl EntityKind {
    fn as_str(&self) -> &'static str {
        match self {
            EntityKind::Job => "job",
            EntityKind::Agent => "agent",
        }
    }
}

/// A resolved entity match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityMatch {
    pub kind: EntityKind,
    pub id: String,
    /// Human-readable label (e.g. job name, agent step name)
    pub label: Option<String>,
}

/// Resolve an ID across all entity types.
///
/// Returns all matches. Exact matches take priority over prefix matches:
/// if any exact match is found, prefix matches are discarded.
pub async fn resolve_entity(client: &DaemonClient, query: &str) -> Result<Vec<EntityMatch>> {
    let jobs = client.list_jobs().await?;
    let agents = client.list_agents(None, None).await?;
    Ok(resolve_from_lists(query, &jobs, &agents))
}

/// Pure function for entity resolution — testable without async client.
pub fn resolve_from_lists(
    query: &str,
    jobs: &[oj_wire::JobSummary],
    agents: &[oj_wire::AgentSummary],
) -> Vec<EntityMatch> {
    let mut exact = Vec::new();
    let mut prefix = Vec::new();

    let mut check = |kind: EntityKind, id: &str, label: Option<String>| {
        if id == query {
            exact.push(EntityMatch { kind: kind.clone(), id: id.to_string(), label });
        } else if oj_core::id::prefix_matches(id, query) {
            prefix.push(EntityMatch { kind, id: id.to_string(), label });
        }
    };

    for p in jobs {
        check(EntityKind::Job, &p.id, Some(p.name.clone()));
    }
    for a in agents {
        check(EntityKind::Agent, &a.agent_id, a.agent_name.clone());
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
        eprintln!("  oj {} {} {}  {}", m.kind.as_str(), command_name, m.id, label);
    }
}

// ── Convenience command handlers ──────────────────────────────────────────

pub async fn handle_peek(client: &DaemonClient, id: &str, format: OutputFormat) -> Result<()> {
    let entity = resolve_one(client, id, "peek").await?;
    match entity.kind {
        EntityKind::Job => {
            super::job::handle(super::job::JobCommand::Peek { id: entity.id }, client, None, format)
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
    }
}

pub async fn handle_attach(client: &DaemonClient, id: &str) -> Result<()> {
    let entity = resolve_one(client, id, "attach").await?;
    match entity.kind {
        EntityKind::Job => {
            super::job::handle(
                super::job::JobCommand::Attach { id: entity.id },
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
        EntityKind::Job => {
            super::job::handle(
                super::job::JobCommand::Logs { id: entity.id, follow, limit },
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
        EntityKind::Job => {
            super::job::handle(
                super::job::JobCommand::Show { id: entity.id, verbose },
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
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
