// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj workspace` - Workspace management commands

use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::{ClientKind, DaemonClient};
use crate::color;
use crate::output::{
    apply_limit, filter_by_project, format_or_json, handle_list_with_limit, print_prune_results,
    OutputFormat,
};
use crate::table::{Column, Table};

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
    project: &str,
    project_filter: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    match command {
        WorkspaceCommand::List { limit, no_limit } => {
            let mut workspaces = client.list_workspaces().await?;
            filter_by_project(&mut workspaces, project_filter, |w| &w.project);
            workspaces.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
            let truncation = apply_limit(&mut workspaces, limit, no_limit);

            handle_list_with_limit(
                format,
                &workspaces,
                "No workspaces",
                truncation,
                |items, out| {
                    let cols = vec![
                        Column::muted("ID").with_max(8),
                        Column::left("PROJECT"),
                        Column::left("PATH").with_max(60),
                        Column::left("BRANCH"),
                        Column::status("STATUS"),
                    ];
                    let mut table = Table::new(cols);
                    for w in items {
                        let ns = if w.project.is_empty() { "-" } else { &w.project };
                        let cells = vec![
                            oj_core::short(&w.id, 8).to_string(),
                            ns.to_string(),
                            w.path.display().to_string(),
                            w.branch.as_deref().unwrap_or("-").to_string(),
                            w.status.clone(),
                        ];
                        table.row(cells);
                    }
                    table.render(out);
                },
            )?;
        }
        WorkspaceCommand::Show { id } => {
            let workspace = client.get_workspace(&id).await?;
            format_or_json(format, &workspace, || {
                if let Some(w) = &workspace {
                    println!("{} {}", color::header("Workspace:"), w.id);
                    println!("  {} {}", color::context("Path:"), w.path.display());
                    if let Some(branch) = &w.branch {
                        println!("  {} {}", color::context("Branch:"), branch);
                    }
                    println!("  {} {}", color::context("Owner:"), w.owner);
                    println!("  {} {}", color::context("Status:"), color::status(&w.status));
                } else {
                    println!("Workspace not found: {}", id);
                }
            })?;
        }
        WorkspaceCommand::Drop { id, failed, all } => {
            let request = if all {
                oj_wire::Request::WorkspaceDropAll
            } else if failed {
                oj_wire::Request::WorkspaceDropFailed
            } else if let Some(id) = id {
                oj_wire::Request::WorkspaceDrop { id: id.to_string() }
            } else {
                anyhow::bail!("specify a workspace ID, --failed, or --all");
            };
            let dropped = client.workspace_drop(request).await?;

            format_or_json(format, &dropped, || {
                if dropped.is_empty() {
                    println!("No workspaces deleted");
                    return;
                }

                for ws in &dropped {
                    println!(
                        "Dropping {} ({})",
                        ws.branch.as_deref().unwrap_or(oj_core::short(&ws.id, 8)),
                        ws.path.display()
                    );
                }
            })?;

            // Poll for removal in text mode (after format_or_json prints JSON or text)
            if matches!(format, OutputFormat::Text) && !dropped.is_empty() {
                let ids: Vec<&str> = dropped.iter().map(|ws| ws.id.as_str()).collect();
                poll_workspace_removal(client, &ids).await;
            }
        }
        WorkspaceCommand::Prune { all, dry_run } => {
            let ns = oj_core::namespace_to_option(project);
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
                        ws.branch.as_deref().unwrap_or(oj_core::short(&ws.id, 8)),
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
