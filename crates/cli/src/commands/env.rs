// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! `oj env` — manage environment variables injected into spawned processes.

use anyhow::{bail, Result};
use clap::{Args, Subcommand};

use crate::daemon_process;
use crate::output::OutputFormat;

#[derive(Args)]
pub struct EnvArgs {
    #[command(subcommand)]
    pub command: EnvCommand,
}

#[derive(Subcommand)]
pub enum EnvCommand {
    /// Set an environment variable
    Set {
        /// Variable name
        key: String,
        /// Variable value
        value: String,
        /// Set globally (all projects)
        #[arg(long, conflicts_with = "project")]
        global: bool,
        /// Set for a specific project
        #[arg(long)]
        project: Option<String>,
    },
    /// List environment variables
    List {
        /// Show only global variables
        #[arg(long, conflicts_with = "project")]
        global: bool,
        /// Show only variables for a specific project
        #[arg(long)]
        project: Option<String>,
    },
    /// Remove an environment variable
    Unset {
        /// Variable name
        key: String,
        /// Remove from global scope
        #[arg(long, conflicts_with = "project")]
        global: bool,
        /// Remove from a specific project
        #[arg(long)]
        project: Option<String>,
    },
}

pub fn handle(command: EnvCommand, format: OutputFormat) -> Result<()> {
    let state_dir = daemon_process::daemon_dir()?;

    match command {
        EnvCommand::Set {
            key,
            value,
            global,
            project,
        } => handle_set(&state_dir, &key, &value, global, project.as_deref(), format),
        EnvCommand::List { global, project } => {
            handle_list(&state_dir, global, project.as_deref(), format)
        }
        EnvCommand::Unset {
            key,
            global,
            project,
        } => handle_unset(&state_dir, &key, global, project.as_deref(), format),
    }
}

fn handle_set(
    state_dir: &std::path::Path,
    key: &str,
    value: &str,
    global: bool,
    project: Option<&str>,
    _format: OutputFormat,
) -> Result<()> {
    let path = resolve_scope(state_dir, global, project)?;
    let mut vars = oj_engine::env::read_env_file(&path)?;
    vars.insert(key.to_string(), value.to_string());
    oj_engine::env::write_env_file(&path, &vars)?;
    Ok(())
}

fn handle_unset(
    state_dir: &std::path::Path,
    key: &str,
    global: bool,
    project: Option<&str>,
    _format: OutputFormat,
) -> Result<()> {
    let path = resolve_scope(state_dir, global, project)?;
    let mut vars = oj_engine::env::read_env_file(&path)?;
    if vars.remove(key).is_none() {
        eprintln!("warning: variable '{}' not found", key);
    }
    oj_engine::env::write_env_file(&path, &vars)?;
    Ok(())
}

fn handle_list(
    state_dir: &std::path::Path,
    global: bool,
    project: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    if global {
        let vars = oj_engine::env::read_env_file(&oj_engine::env::global_env_path(state_dir))?;
        return print_scoped_vars(&vars, format);
    }

    if let Some(name) = project {
        let vars =
            oj_engine::env::read_env_file(&oj_engine::env::project_env_path(state_dir, name))?;
        return print_scoped_vars(&vars, format);
    }

    // List all: global + all discovered project env files
    let global_vars = oj_engine::env::read_env_file(&oj_engine::env::global_env_path(state_dir))?;
    let mut projects: Vec<(String, std::collections::BTreeMap<String, String>)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(state_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(project_name) = name_str.strip_prefix("env.") {
                if !project_name.is_empty() {
                    if let Ok(vars) = oj_engine::env::read_env_file(&entry.path()) {
                        if !vars.is_empty() {
                            projects.push((project_name.to_string(), vars));
                        }
                    }
                }
            }
        }
    }
    projects.sort_by(|(a, _), (b, _)| a.cmp(b));

    match format {
        OutputFormat::Text => {
            if global_vars.is_empty() && projects.is_empty() {
                println!("No environment variables configured");
                return Ok(());
            }

            if !global_vars.is_empty() {
                println!("# global");
                for (k, v) in &global_vars {
                    println!("{k}={v}");
                }
            }

            for (name, vars) in &projects {
                if !global_vars.is_empty() || projects.len() > 1 {
                    println!();
                }
                println!("# project: {name}");
                for (k, v) in vars {
                    println!("{k}={v}");
                }
            }
        }
        OutputFormat::Json => {
            let mut project_map = serde_json::Map::new();
            for (name, vars) in &projects {
                let obj: serde_json::Value = vars
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect::<serde_json::Map<_, _>>()
                    .into();
                project_map.insert(name.clone(), obj);
            }

            let output = serde_json::json!({
                "global": global_vars.iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect::<serde_json::Map<_, _>>(),
                "projects": project_map,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }

    Ok(())
}

fn print_scoped_vars(
    vars: &std::collections::BTreeMap<String, String>,
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Text => {
            for (k, v) in vars {
                println!("{k}={v}");
            }
        }
        OutputFormat::Json => {
            let obj: serde_json::Value = vars
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect::<serde_json::Map<_, _>>()
                .into();
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
    }
    Ok(())
}

/// Resolve the env file path from the --global / --project flags.
fn resolve_scope(
    state_dir: &std::path::Path,
    global: bool,
    project: Option<&str>,
) -> Result<std::path::PathBuf> {
    match (global, project) {
        (true, _) => Ok(oj_engine::env::global_env_path(state_dir)),
        (false, Some(name)) => Ok(oj_engine::env::project_env_path(state_dir, name)),
        (false, None) => bail!("one of --global or --project is required"),
    }
}
