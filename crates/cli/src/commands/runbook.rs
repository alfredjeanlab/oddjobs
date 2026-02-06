// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj runbook` — inspect runbooks and discover libraries.

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::Path;

use crate::output::OutputFormat;
use crate::table::{Column, Table};

#[derive(Args)]
pub struct RunbookArgs {
    #[command(subcommand)]
    pub command: RunbookCommand,
}

#[derive(Subcommand)]
pub enum RunbookCommand {
    /// List runbooks for the current project
    List {},
    /// Search available libraries to import
    Search {
        /// Filter by name or description
        query: Option<String>,
    },
    /// Show library contents and required parameters
    Info {
        /// Library path (e.g. "oj/wok")
        path: String,
    },
}

pub fn handle(command: RunbookCommand, project_root: &Path, format: OutputFormat) -> Result<()> {
    match command {
        RunbookCommand::List {} => handle_list(project_root, format),
        RunbookCommand::Search { query } => handle_search(query.as_deref(), format),
        RunbookCommand::Info { path } => handle_show(&path, format),
    }
}

fn handle_list(project_root: &Path, format: OutputFormat) -> Result<()> {
    let runbook_dir = project_root.join(".oj/runbooks");
    let summaries = oj_runbook::collect_runbook_summaries(&runbook_dir)?;

    if summaries.is_empty() {
        eprintln!("No runbooks found in {}", runbook_dir.display());
        return Ok(());
    }

    match format {
        OutputFormat::Text => {
            let mut table = Table::new(vec![
                Column::left("FILE"),
                Column::left("IMPORTS"),
                Column::left("COMMANDS"),
                Column::left("DESCRIPTION").with_max(60),
            ]);

            for summary in &summaries {
                let imports = if summary.imports.is_empty() {
                    "-".to_string()
                } else {
                    summary
                        .imports
                        .keys()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };

                let commands = if summary.commands.is_empty() {
                    // Show imported command names if no local commands
                    let imported_cmds = imported_command_names(&summary.imports);
                    if imported_cmds.is_empty() {
                        "-".to_string()
                    } else {
                        format!("{} (imported)", imported_cmds.join(", "))
                    }
                } else {
                    summary.commands.join(", ")
                };

                let description = summary.description.as_deref().unwrap_or("");

                table.row(vec![
                    summary.file.clone(),
                    imports,
                    commands,
                    description.to_string(),
                ]);
            }

            table.render(&mut std::io::stdout());
        }
        OutputFormat::Json => {
            let entries: Vec<serde_json::Value> = summaries
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "file": s.file,
                        "imports": s.imports.keys().collect::<Vec<_>>(),
                        "commands": s.commands,
                        "jobs": s.jobs,
                        "agents": s.agents,
                        "queues": s.queues,
                        "workers": s.workers,
                        "crons": s.crons,
                        "description": s.description,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
    }

    Ok(())
}

/// Resolve command names from imports by parsing each library.
fn imported_command_names(
    imports: &std::collections::HashMap<String, oj_runbook::ImportDef>,
) -> Vec<String> {
    let mut names = Vec::new();
    for (source, import_def) in imports {
        let files = match oj_runbook::resolve_library(source) {
            Ok(f) => f,
            Err(_) => continue,
        };
        for (_, content) in files {
            let runbook =
                match oj_runbook::parse_runbook_with_format(content, oj_runbook::Format::Hcl) {
                    Ok(rb) => rb,
                    Err(_) => continue,
                };
            let prefix = import_def.alias.as_deref();
            for cmd_name in runbook.commands.keys() {
                let name = match prefix {
                    Some(p) => format!("{}:{}", p, cmd_name),
                    None => cmd_name.clone(),
                };
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

fn handle_search(query: Option<&str>, format: OutputFormat) -> Result<()> {
    let libraries = oj_runbook::available_libraries();

    let filtered: Vec<_> = match query {
        Some(q) => {
            let q_lower = q.to_lowercase();
            libraries
                .into_iter()
                .filter(|lib| {
                    lib.source.to_lowercase().contains(&q_lower)
                        || lib.description.to_lowercase().contains(&q_lower)
                })
                .collect()
        }
        None => libraries,
    };

    if filtered.is_empty() {
        if let Some(q) = query {
            eprintln!("No libraries matching '{}'", q);
        } else {
            eprintln!("No libraries available");
        }
        return Ok(());
    }

    match format {
        OutputFormat::Text => {
            let mut table = Table::new(vec![
                Column::left("LIBRARY"),
                Column::left("CONSTS"),
                Column::left("DESCRIPTION").with_max(60),
            ]);

            for lib in &filtered {
                let consts_display = format_const_summary(lib.files);
                table.row(vec![
                    lib.source.to_string(),
                    consts_display,
                    lib.description.clone(),
                ]);
            }

            table.render(&mut std::io::stdout());
        }
        OutputFormat::Json => {
            let entries: Vec<serde_json::Value> = filtered
                .iter()
                .map(|lib| {
                    let consts_json = format_consts_json(lib.files);
                    serde_json::json!({
                        "source": lib.source,
                        "description": lib.description,
                        "consts": consts_json,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
    }

    Ok(())
}

fn handle_show(path: &str, format: OutputFormat) -> Result<()> {
    let files = oj_runbook::resolve_library(path).map_err(|_| {
        anyhow::anyhow!(
            "unknown library '{}'; use 'oj runbook search' to see available libraries",
            path
        )
    })?;

    let description = files
        .first()
        .and_then(|(_, content)| oj_runbook::extract_file_comment(content))
        .map(|c| c.short)
        .unwrap_or_default();

    // Collect const definitions and merge all files into a single runbook
    let const_defs = extract_const_defs(files);
    let runbook = merge_library_files(files)?;

    match format {
        OutputFormat::Text => {
            println!("Library: {}", path);
            if !description.is_empty() {
                println!("{}", description);
            }

            if !const_defs.is_empty() {
                println!("\nParameters:");
                let mut sorted_consts: Vec<_> = const_defs.iter().collect();
                sorted_consts.sort_by_key(|(name, _)| *name);
                for (name, def) in &sorted_consts {
                    let req = if def.default.is_none() {
                        "(required)"
                    } else {
                        "(optional)"
                    };
                    let default_str = match &def.default {
                        Some(d) => format!(" [default: \"{}\"]", d),
                        None => String::new(),
                    };
                    println!("  {:<12} {:<12} {}", name, req, default_str.trim());
                }
            }

            let mut entity_lines = Vec::new();
            if !runbook.commands.is_empty() {
                let mut names: Vec<_> = runbook.commands.keys().collect();
                names.sort();
                entity_lines.push(format!(
                    "  Commands:  {}",
                    names.into_iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            if !runbook.jobs.is_empty() {
                let mut names: Vec<_> = runbook.jobs.keys().collect();
                names.sort();
                entity_lines.push(format!(
                    "  Jobs:      {}",
                    names.into_iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            if !runbook.agents.is_empty() {
                let mut names: Vec<_> = runbook.agents.keys().collect();
                names.sort();
                entity_lines.push(format!(
                    "  Agents:    {}",
                    names.into_iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            if !runbook.queues.is_empty() {
                let mut names: Vec<_> = runbook.queues.keys().collect();
                names.sort();
                entity_lines.push(format!(
                    "  Queues:    {}",
                    names.into_iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            if !runbook.workers.is_empty() {
                let mut names: Vec<_> = runbook.workers.keys().collect();
                names.sort();
                entity_lines.push(format!(
                    "  Workers:   {}",
                    names.into_iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            if !runbook.crons.is_empty() {
                let mut names: Vec<_> = runbook.crons.keys().collect();
                names.sort();
                entity_lines.push(format!(
                    "  Crons:     {}",
                    names.into_iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }

            if !entity_lines.is_empty() {
                println!("\nEntities:");
                for line in &entity_lines {
                    println!("{}", line);
                }
            }

            // Usage example
            println!("\nUsage:");
            if const_defs.values().any(|c| c.default.is_none()) {
                println!("  import \"{}\" {{", path);
                let mut required: Vec<_> = const_defs
                    .iter()
                    .filter(|(_, c)| c.default.is_none())
                    .map(|(name, _)| name.as_str())
                    .collect();
                required.sort();
                for name in required {
                    println!("    const \"{}\" {{ value = \"...\" }}", name);
                }
                println!("  }}");
            } else {
                println!("  import \"{}\" {{}}", path);
            }
        }
        OutputFormat::Json => {
            let mut consts_json: Vec<serde_json::Value> = const_defs
                .iter()
                .map(|(name, c)| {
                    serde_json::json!({
                        "name": name,
                        "required": c.default.is_none(),
                        "default": c.default,
                    })
                })
                .collect();
            consts_json.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));

            let entities = build_entity_map(&runbook);

            let obj = serde_json::json!({
                "source": path,
                "description": description,
                "consts": consts_json,
                "entities": entities,
            });
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
    }

    Ok(())
}

/// Build a JSON map of entity types to sorted name lists.
fn build_entity_map(runbook: &oj_runbook::Runbook) -> serde_json::Value {
    fn sorted<V>(map: &std::collections::HashMap<String, V>) -> Vec<String> {
        let mut keys: Vec<_> = map.keys().cloned().collect();
        keys.sort();
        keys
    }
    fn insert<V>(
        m: &mut serde_json::Map<String, serde_json::Value>,
        key: &str,
        map: &std::collections::HashMap<String, V>,
    ) {
        if !map.is_empty() {
            m.insert(key.to_string(), serde_json::json!(sorted(map)));
        }
    }
    let mut m = serde_json::Map::new();
    insert(&mut m, "commands", &runbook.commands);
    insert(&mut m, "jobs", &runbook.jobs);
    insert(&mut m, "agents", &runbook.agents);
    insert(&mut m, "queues", &runbook.queues);
    insert(&mut m, "workers", &runbook.workers);
    insert(&mut m, "crons", &runbook.crons);
    serde_json::Value::Object(m)
}

/// Format const definitions as a JSON array.
fn format_consts_json(files: &[(&str, &str)]) -> Vec<serde_json::Value> {
    let consts = extract_const_defs(files);
    let mut result: Vec<serde_json::Value> = consts
        .iter()
        .map(|(name, c)| {
            serde_json::json!({
                "name": name,
                "required": c.default.is_none(),
                "default": c.default,
            })
        })
        .collect();
    result.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    result
}

/// Extract const definitions from all library files.
fn extract_const_defs(
    files: &[(&str, &str)],
) -> std::collections::HashMap<String, oj_runbook::ConstDef> {
    let mut all_consts = std::collections::HashMap::new();
    for (_, content) in files {
        if let Ok(runbook) = oj_runbook::parse_runbook_with_format(content, oj_runbook::Format::Hcl)
        {
            all_consts.extend(runbook.consts);
        }
    }
    all_consts
}

/// Merge all library files into a single runbook for entity enumeration.
fn merge_library_files(files: &[(&str, &str)]) -> Result<oj_runbook::Runbook> {
    let mut merged = oj_runbook::Runbook::default();
    for (_, content) in files {
        let runbook = oj_runbook::parse_runbook_with_format(content, oj_runbook::Format::Hcl)?;
        // Simple merge — library files shouldn't conflict
        merged.commands.extend(runbook.commands);
        merged.jobs.extend(runbook.jobs);
        merged.agents.extend(runbook.agents);
        merged.queues.extend(runbook.queues);
        merged.workers.extend(runbook.workers);
        merged.crons.extend(runbook.crons);
    }
    Ok(merged)
}

/// Format const defs for the search table summary.
fn format_const_summary(files: &[(&str, &str)]) -> String {
    let defs = extract_const_defs(files);
    if defs.is_empty() {
        return "-".to_string();
    }
    let mut items: Vec<_> = defs
        .iter()
        .map(|(name, c)| {
            if c.default.is_none() {
                format!("{} (req)", name)
            } else {
                name.clone()
            }
        })
        .collect();
    items.sort();
    items.join(", ")
}

#[cfg(test)]
#[path = "runbook_tests.rs"]
mod tests;
