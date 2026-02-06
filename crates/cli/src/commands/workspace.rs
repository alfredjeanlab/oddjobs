// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj workspace` - Workspace management commands

use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{Args, Subcommand};

use oj_core::ShortId;

use crate::client::{ClientKind, DaemonClient};
use crate::color;
use crate::output::{print_prune_results, OutputFormat};
use crate::table::{project_cell, should_show_project, Column, Table};

#[derive(Args)]
pub struct WorkspaceArgs {
    #[command(subcommand)]
    pub command: WorkspaceCommand,
}

#[derive(Subcommand)]
pub enum WorkspaceCommand {
    /// List all workspaces
    List {
        /// Maximum number of workspaces to show (default: 20)
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,

        /// Show all workspaces (no limit)
        #[arg(long, conflicts_with = "limit")]
        no_limit: bool,
    },
    /// Show details of a workspace
    Show {
        /// Workspace ID
        id: String,
    },
    /// Delete workspace(s)
    Drop {
        /// Workspace ID (prefix match)
        id: Option<String>,
        /// Delete all failed workspaces
        #[arg(long)]
        failed: bool,
        /// Delete all workspaces
        #[arg(long)]
        all: bool,
    },
    /// Remove workspaces from completed/failed jobs
    Prune {
        /// Remove all terminal workspaces regardless of age
        #[arg(long)]
        all: bool,
        /// Show what would be pruned without doing it
        #[arg(long)]
        dry_run: bool,
    },
}

impl WorkspaceCommand {
    pub fn client_kind(&self) -> ClientKind {
        match self {
            Self::List { .. } | Self::Show { .. } => ClientKind::Query,
            _ => ClientKind::Action,
        }
    }
}

pub async fn handle(
    command: WorkspaceCommand,
    client: &DaemonClient,
    namespace: &str,
    project_filter: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        WorkspaceCommand::List { limit, no_limit } => {
            let mut workspaces = client.list_workspaces().await?;

            // Filter by explicit --project flag (OJ_NAMESPACE is NOT used for filtering)
            if let Some(proj) = project_filter {
                workspaces.retain(|w| w.namespace == proj);
            }

            // Sort by recency (most recent first)
            workspaces.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));

            // Limit
            let total = workspaces.len();
            let effective_limit = if no_limit { total } else { limit };
            let truncated = total > effective_limit;
            if truncated {
                workspaces.truncate(effective_limit);
            }

            match format {
                OutputFormat::Text => {
                    if workspaces.is_empty() {
                        println!("No workspaces");
                    } else {
                        let show_project =
                            should_show_project(workspaces.iter().map(|w| w.namespace.as_str()));

                        let mut cols = vec![Column::muted("ID").with_max(8)];
                        if show_project {
                            cols.push(Column::left("PROJECT"));
                        }
                        cols.extend([
                            Column::left("PATH").with_max(60),
                            Column::left("BRANCH"),
                            Column::status("STATUS"),
                        ]);
                        let mut table = Table::new(cols);

                        for w in &workspaces {
                            let mut cells = vec![w.id.short(8).to_string()];
                            if show_project {
                                cells.push(project_cell(&w.namespace));
                            }
                            cells.extend([
                                w.path.display().to_string(),
                                w.branch.as_deref().unwrap_or("-").to_string(),
                                w.status.clone(),
                            ]);
                            table.row(cells);
                        }
                        table.render(&mut std::io::stdout());
                    }

                    if truncated {
                        let remaining = total - effective_limit;
                        println!(
                            "\n... {} more not shown. Use --no-limit or --limit N to see more.",
                            remaining
                        );
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&workspaces)?);
                }
            }
        }
        WorkspaceCommand::Show { id } => {
            let workspace = client.get_workspace(&id).await?;

            match format {
                OutputFormat::Text => {
                    if let Some(w) = workspace {
                        println!("{} {}", color::header("Workspace:"), w.id);
                        println!("  {} {}", color::context("Path:"), w.path.display());
                        if let Some(branch) = &w.branch {
                            println!("  {} {}", color::context("Branch:"), branch);
                        }
                        if let Some(owner) = &w.owner {
                            println!("  {} {}", color::context("Owner:"), owner);
                        }
                        println!(
                            "  {} {}",
                            color::context("Status:"),
                            color::status(&w.status)
                        );
                    } else {
                        println!("Workspace not found: {}", id);
                    }
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&workspace)?);
                }
            }
        }
        WorkspaceCommand::Drop { id, failed, all } => {
            let dropped = if all {
                client.workspace_drop_all().await?
            } else if failed {
                client.workspace_drop_failed().await?
            } else if let Some(id) = id {
                client.workspace_drop(&id).await?
            } else {
                anyhow::bail!("specify a workspace ID, --failed, or --all");
            };

            match format {
                OutputFormat::Text => {
                    if dropped.is_empty() {
                        println!("No workspaces deleted");
                        return Ok(());
                    }

                    for ws in &dropped {
                        println!(
                            "Dropping {} ({})",
                            ws.branch.as_deref().unwrap_or(ws.id.short(8)),
                            ws.path.display()
                        );
                    }

                    let ids: Vec<&str> = dropped.iter().map(|ws| ws.id.as_str()).collect();
                    poll_workspace_removal(client, &ids).await;
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&dropped)?);
                }
            }
        }
        WorkspaceCommand::Prune { all, dry_run } => {
            let ns = oj_core::namespace_to_option(namespace);
            let (pruned, skipped) = client.workspace_prune(all, dry_run, ns).await?;

            print_prune_results(
                &pruned,
                skipped,
                dry_run,
                format,
                "workspace",
                "active workspace(s) skipped",
                |ws| {
                    format!(
                        "{} ({})",
                        ws.branch.as_deref().unwrap_or(ws.id.short(8)),
                        ws.path.display()
                    )
                },
            )?;
        }
    }

    Ok(())
}

/// Poll daemon state to confirm workspaces have been removed.
///
/// Checks every 500ms for up to 10s. Ctrl+C exits the poll
/// without cancelling the drop operation.
async fn poll_workspace_removal(client: &DaemonClient, ids: &[&str]) {
    let poll_interval = Duration::from_millis(500);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut confirmed = false;

    println!("\nWaiting for removal... (Ctrl+C to skip)");

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                break;
            }
            _ = tokio::time::sleep(poll_interval) => {
                if Instant::now() >= deadline {
                    break;
                }
                if let Ok(workspaces) = client.list_workspaces().await {
                    let any_remaining = ids.iter().any(|id| {
                        workspaces.iter().any(|w| w.id == *id)
                    });
                    if !any_remaining {
                        confirmed = true;
                        break;
                    }
                }
            }
        }
    }

    if confirmed {
        println!("Deleted {} workspace(s)", ids.len());
    } else {
        println!("Still processing, check: oj workspace list");
    }
}
