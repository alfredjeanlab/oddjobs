// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj runbook` — inspect runbooks and discover libraries.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

use crate::output::{format_or_json, OutputFormat};
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
    /// Install an HCL library for use in runbook imports
    Add {
        /// Path to an .hcl file or directory of .hcl files
        path: String,
        /// Library name (default: inferred from source path)
        #[arg(long)]
        name: Option<String>,
        /// Install to project-level .oj/libraries/ instead of user-level
        #[arg(long)]
        project: bool,
    },
}

pub fn handle(command: RunbookCommand, project_path: &Path, format: OutputFormat) -> Result<()> {
    let lib_dirs = library_dirs(project_path);
    match command {
        RunbookCommand::List {} => handle_list(project_path, format),
        RunbookCommand::Search { query } => handle_search(query.as_deref(), &lib_dirs, format),
        RunbookCommand::Info { path } => handle_show(&path, &lib_dirs, format),
        RunbookCommand::Add { path, name, project } => {
            handle_add(&path, name.as_deref(), project, project_path)
        }
    }
}

/// Compute library directories for resolution: project-level then user-level.
fn library_dirs(project_path: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let project_libs = project_path.join(".oj/libraries");
    if project_libs.is_dir() {
        dirs.push(project_libs);
    }
    if let Ok(state_dir) = crate::env::state_dir() {
        let user_libs = state_dir.join("libraries");
        if user_libs.is_dir() {
            dirs.push(user_libs);
        }
    }
    dirs
}

fn handle_list(project_path: &Path, format: OutputFormat) -> Result<()> {
    let runbook_dir = project_path.join(".oj/runbooks");
    let lib_dirs = library_dirs(project_path);
    let summaries = oj_runbook::collect_runbook_summaries(&runbook_dir)?;

    if summaries.is_empty() {
        eprintln!("No runbooks found in {}", runbook_dir.display());
        return Ok(());
    }

    let json_data: Vec<serde_json::Value> = summaries
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
    format_or_json(format, &json_data, || {
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
                summary.imports.keys().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            };

            let commands = if summary.commands.is_empty() {
                let imported_cmds = imported_command_names(&summary.imports, &lib_dirs);
                if imported_cmds.is_empty() {
                    "-".to_string()
                } else {
                    format!("{} (imported)", imported_cmds.join(", "))
                }
            } else {
                summary.commands.join(", ")
            };

            table.row(vec![
                summary.file.clone(),
                imports,
                commands,
                summary.description.as_deref().unwrap_or("").to_string(),
            ]);
        }

        table.render(&mut std::io::stdout());
    })
}

/// Resolve command names from imports by parsing each library.
fn imported_command_names(
    imports: &std::collections::HashMap<String, oj_runbook::ImportDef>,
    library_dirs: &[PathBuf],
) -> Vec<String> {
    let mut names = Vec::new();
    for (source, import_def) in imports {
        let files = match oj_runbook::resolve_library(source, library_dirs) {
            Ok(f) => f,
            Err(_) => continue,
        };
        for (_, content) in &files {
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

fn handle_search(
    query: Option<&str>,
    library_dirs: &[PathBuf],
    format: OutputFormat,
) -> Result<()> {
    let libraries = oj_runbook::available_libraries(library_dirs);

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

    let json_data: Vec<serde_json::Value> = filtered
        .iter()
        .map(|lib| {
            serde_json::json!({
                "source": lib.source,
                "description": lib.description,
                "consts": format_consts_json(&lib.files),
            })
        })
        .collect();
    format_or_json(format, &json_data, || {
        let mut table = Table::new(vec![
            Column::left("LIBRARY"),
            Column::left("CONSTS"),
            Column::left("DESCRIPTION").with_max(60),
        ]);

        for lib in &filtered {
            table.row(vec![
                lib.source.clone(),
                format_const_summary(&lib.files),
                lib.description.clone(),
            ]);
        }

        table.render(&mut std::io::stdout());
    })
}

fn handle_show(path: &str, library_dirs: &[PathBuf], format: OutputFormat) -> Result<()> {
    let files = oj_runbook::resolve_library(path, library_dirs).map_err(|_| {
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

    let const_defs = extract_const_defs(&files);
    let runbook = merge_library_files(&files)?;
    let consts_json = const_defs_to_json(&const_defs);

    let obj = serde_json::json!({
        "source": path,
        "description": &description,
        "consts": consts_json,
        "entities": build_entity_map(&runbook),
    });
    format_or_json(format, &obj, || {
        println!("Library: {}", path);
        if !description.is_empty() {
            println!("{}", description);
        }

        if !const_defs.is_empty() {
            println!("\nParameters:");
            let mut sorted_consts: Vec<_> = const_defs.iter().collect();
            sorted_consts.sort_by_key(|(name, _)| *name);
            for (name, def) in &sorted_consts {
                let req = if def.default.is_none() { "(required)" } else { "(optional)" };
                let default_str = match &def.default {
                    Some(d) => format!(" [default: \"{}\"]", d),
                    None => String::new(),
                };
                println!("  {:<12} {:<12} {}", name, req, default_str.trim());
            }
        }

        let entity_lines = collect_entity_lines(&runbook);
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
    })
}

/// Sorted keys from a HashMap.
fn sorted_keys<V>(map: &std::collections::HashMap<String, V>) -> Vec<String> {
    let mut keys: Vec<_> = map.keys().cloned().collect();
    keys.sort();
    keys
}

/// Collect (label, sorted-keys) pairs for all non-empty entity types.
fn runbook_entity_pairs(runbook: &oj_runbook::Runbook) -> Vec<(&'static str, Vec<String>)> {
    let candidates: Vec<(&str, Vec<String>)> = vec![
        ("commands", sorted_keys(&runbook.commands)),
        ("jobs", sorted_keys(&runbook.jobs)),
        ("agents", sorted_keys(&runbook.agents)),
        ("queues", sorted_keys(&runbook.queues)),
        ("workers", sorted_keys(&runbook.workers)),
        ("crons", sorted_keys(&runbook.crons)),
    ];
    candidates.into_iter().filter(|(_, names)| !names.is_empty()).collect()
}

/// Collect formatted entity lines for text display.
fn collect_entity_lines(runbook: &oj_runbook::Runbook) -> Vec<String> {
    // Capitalize first letter of label for display
    runbook_entity_pairs(runbook)
        .into_iter()
        .map(|(label, names)| {
            let cap = format!("{}{}:", &label[..1].to_uppercase(), &label[1..]);
            format!("  {:<11}{}", cap, names.join(", "))
        })
        .collect()
}

/// Build a JSON map of entity types to sorted name lists.
fn build_entity_map(runbook: &oj_runbook::Runbook) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (key, names) in runbook_entity_pairs(runbook) {
        m.insert(key.to_string(), serde_json::json!(names));
    }
    serde_json::Value::Object(m)
}

/// Convert const definitions to a sorted JSON array.
fn const_defs_to_json(
    consts: &std::collections::HashMap<String, oj_runbook::ConstDef>,
) -> Vec<serde_json::Value> {
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

/// Format const definitions from library files as a JSON array.
fn format_consts_json(files: &[(String, String)]) -> Vec<serde_json::Value> {
    const_defs_to_json(&extract_const_defs(files))
}

/// Extract const definitions from all library files.
fn extract_const_defs(
    files: &[(String, String)],
) -> std::collections::HashMap<String, oj_runbook::ConstDef> {
    let mut all_consts = std::collections::HashMap::new();
    for (_, content) in files {
        // Strip %{...} const directives before parsing to avoid shell validation
        // errors on template content (const defs are never inside conditional blocks)
        let stripped = match oj_runbook::strip_const_directives(content) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if let Ok(runbook) =
            oj_runbook::parse_runbook_with_format(&stripped, oj_runbook::Format::Hcl)
        {
            all_consts.extend(runbook.consts);
        }
    }
    all_consts
}

/// Merge all library files into a single runbook for entity enumeration.
fn merge_library_files(files: &[(String, String)]) -> Result<oj_runbook::Runbook> {
    let mut merged = oj_runbook::Runbook::default();
    for (_, content) in files {
        // Strip %{...} const directives before parsing to avoid shell validation
        // errors on template content
        let stripped = oj_runbook::strip_const_directives(content)
            .map_err(|msg| anyhow::anyhow!("{}", msg))?;
        let runbook = oj_runbook::parse_runbook_with_format(&stripped, oj_runbook::Format::Hcl)?;
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
fn format_const_summary(files: &[(String, String)]) -> String {
    let defs = extract_const_defs(files);
    if defs.is_empty() {
        return "-".to_string();
    }
    let mut items: Vec<_> = defs
        .iter()
        .map(|(name, c)| if c.default.is_none() { format!("{} (req)", name) } else { name.clone() })
        .collect();
    items.sort();
    items.join(", ")
}

fn handle_add(
    source_path: &str,
    name: Option<&str>,
    project_level: bool,
    project_path: &Path,
) -> Result<()> {
    // Resolve source path
    let source = PathBuf::from(source_path);
    let source = if source.is_absolute() { source } else { std::env::current_dir()?.join(source) };
    let source = source
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve '{}': {}", source_path, e))?;

    // Infer library name
    let lib_name = match name {
        Some(n) => n.to_string(),
        None => {
            if source.is_dir() {
                source
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("cannot infer library name from path"))?
                    .to_string_lossy()
                    .to_string()
            } else {
                source
                    .file_stem()
                    .ok_or_else(|| anyhow::anyhow!("cannot infer library name from path"))?
                    .to_string_lossy()
                    .to_string()
            }
        }
    };

    // Validate name
    if lib_name.is_empty() || lib_name.contains("..") {
        bail!("invalid library name '{}'", lib_name);
    }
    if !lib_name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '/') {
        bail!("invalid library name '{}': only alphanumeric, '-', '_', '/' allowed", lib_name);
    }

    // Determine target directory
    let target_dir = if project_level {
        project_path.join(".oj/libraries").join(&lib_name)
    } else {
        let state_dir = crate::env::state_dir()
            .map_err(|e| anyhow::anyhow!("cannot determine state directory: {}", e))?;
        state_dir.join("libraries").join(&lib_name)
    };

    // Remove existing if present
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)?;
    }
    std::fs::create_dir_all(&target_dir)?;

    // Copy .hcl files
    let mut copied = 0;
    if source.is_dir() {
        for entry in std::fs::read_dir(&source)?.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "hcl") {
                let Some(filename) = path.file_name() else {
                    continue;
                };
                std::fs::copy(&path, target_dir.join(filename))?;
                copied += 1;
            }
        }
    } else if source.extension().is_some_and(|e| e == "hcl") {
        let Some(filename) = source.file_name() else {
            bail!("invalid source path: {}", source.display());
        };
        std::fs::copy(&source, target_dir.join(filename))?;
        copied = 1;
    } else {
        bail!("source must be an .hcl file or directory of .hcl files: {}", source.display());
    }

    if copied == 0 {
        // Clean up empty dir
        let _ = std::fs::remove_dir(&target_dir);
        bail!("no .hcl files found in {}", source.display());
    }

    // Validate the installed library parses
    for entry in std::fs::read_dir(&target_dir)?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "hcl") {
            let content = std::fs::read_to_string(&path)?;
            let stripped = oj_runbook::strip_const_directives(&content).unwrap_or(content.clone());
            if let Err(e) =
                oj_runbook::parse_runbook_with_format(&stripped, oj_runbook::Format::Hcl)
            {
                eprintln!(
                    "warning: {} has parse errors: {}",
                    path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default(),
                    e
                );
            }
        }
    }

    let scope = if project_level { "project" } else { "user" };
    println!(
        "Installed library '{}' ({} file{}) to {} ({})",
        lib_name,
        copied,
        if copied == 1 { "" } else { "s" },
        target_dir.display(),
        scope,
    );
    println!("  Usage: import \"{}\" {{}}", lib_name);

    Ok(())
}

#[cfg(test)]
#[path = "runbook_tests.rs"]
mod tests;
