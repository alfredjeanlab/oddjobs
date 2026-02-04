// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj workspace` - Workspace management commands

use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::client::DaemonClient;
use crate::color;
use crate::output::OutputFormat;

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

        /// Filter by project namespace
        #[arg(long = "project")]
        project: Option<String>,
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
    /// Remove workspaces from completed/failed pipelines
    Prune {
        /// Remove all terminal workspaces regardless of age
        #[arg(long)]
        all: bool,
        /// Show what would be pruned without doing it
        #[arg(long)]
        dry_run: bool,
        /// Project namespace override
        #[arg(long = "project")]
        project: Option<String>,
    },
}

pub async fn handle(
    command: WorkspaceCommand,
    client: &DaemonClient,
    format: OutputFormat,
) -> Result<()> {
    match command {
        WorkspaceCommand::List {
            limit,
            no_limit,
            project,
        } => {
            let mut workspaces = client.list_workspaces().await?;

            // Filter by project namespace (empty OJ_NAMESPACE treated as unset)
            let filter_namespace =
                project.or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()));
            if let Some(ref ns) = filter_namespace {
                workspaces.retain(|w| w.namespace == *ns);
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
                        // Determine whether to show PROJECT column
                        let namespaces: std::collections::HashSet<&str> =
                            workspaces.iter().map(|w| w.namespace.as_str()).collect();
                        let show_project =
                            namespaces.len() > 1 || namespaces.iter().any(|n| !n.is_empty());

                        // Compute dynamic column widths from data
                        let id_w = workspaces
                            .iter()
                            .map(|w| w.id.len().min(8))
                            .max()
                            .unwrap_or(2)
                            .max(2);
                        let no_project = "(no project)";
                        let proj_w = if show_project {
                            workspaces
                                .iter()
                                .map(|w| {
                                    if w.namespace.is_empty() {
                                        no_project.len()
                                    } else {
                                        w.namespace.len()
                                    }
                                })
                                .max()
                                .unwrap_or(7)
                                .max(7)
                        } else {
                            0
                        };
                        let path_w = workspaces
                            .iter()
                            .map(|w| w.path.display().to_string().len())
                            .max()
                            .unwrap_or(4)
                            .clamp(4, 60);
                        let branch_w = workspaces
                            .iter()
                            .map(|w| w.branch.as_deref().unwrap_or("-").len())
                            .max()
                            .unwrap_or(6)
                            .max(6);

                        if show_project {
                            println!(
                                "{} {} {} {} {}",
                                color::header(&format!("{:<id_w$}", "ID")),
                                color::header(&format!("{:<proj_w$}", "PROJECT")),
                                color::header(&format!("{:<path_w$}", "PATH")),
                                color::header(&format!("{:<branch_w$}", "BRANCH")),
                                color::header("STATUS"),
                            );
                        } else {
                            println!(
                                "{} {} {} {}",
                                color::header(&format!("{:<id_w$}", "ID")),
                                color::header(&format!("{:<path_w$}", "PATH")),
                                color::header(&format!("{:<branch_w$}", "BRANCH")),
                                color::header("STATUS"),
                            );
                        }
                        for w in &workspaces {
                            let path_str: String =
                                w.path.display().to_string().chars().take(path_w).collect();
                            let branch = w.branch.as_deref().unwrap_or("-");
                            if show_project {
                                let proj = if w.namespace.is_empty() {
                                    no_project
                                } else {
                                    &w.namespace
                                };
                                println!(
                                    "{} {:<proj_w$} {:<path_w$} {:<branch_w$} {}",
                                    color::muted(&format!("{:<id_w$}", &w.id[..8.min(w.id.len())])),
                                    &proj[..proj_w.min(proj.len())],
                                    path_str,
                                    branch,
                                    color::status(&w.status),
                                );
                            } else {
                                println!(
                                    "{} {:<path_w$} {:<branch_w$} {}",
                                    color::muted(&format!("{:<id_w$}", &w.id[..8.min(w.id.len())])),
                                    path_str,
                                    branch,
                                    color::status(&w.status),
                                );
                            }
                        }
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
                            ws.branch.as_deref().unwrap_or(&ws.id[..8.min(ws.id.len())]),
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
        WorkspaceCommand::Prune {
            all,
            dry_run,
            project,
        } => {
            let (pruned, skipped) = client
                .workspace_prune(all, dry_run, project.as_deref())
                .await?;

            match format {
                OutputFormat::Text => {
                    if dry_run {
                        println!("Dry run — no changes made\n");
                    }

                    for ws in &pruned {
                        let label = if dry_run { "Would prune" } else { "Pruned" };
                        println!(
                            "{} {} ({})",
                            label,
                            ws.branch.as_deref().unwrap_or(&ws.id[..8.min(ws.id.len())]),
                            ws.path.display()
                        );
                    }

                    let verb = if dry_run { "would be pruned" } else { "pruned" };
                    println!(
                        "\n{} workspace(s) {}, {} active workspace(s) skipped",
                        pruned.len(),
                        verb,
                        skipped
                    );
                }
                OutputFormat::Json => {
                    let obj = serde_json::json!({
                        "dry_run": dry_run,
                        "pruned": pruned,
                        "skipped": skipped,
                    });
                    println!("{}", serde_json::to_string_pretty(&obj)?);
                }
            }
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
