// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! User-managed environment variable files (dotenv-style).
//!
//! Shared by the CLI (`oj env` commands) and the engine (injection at spawn time).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Resolve the path to the global env file.
pub fn global_env_path(state_dir: &Path) -> PathBuf {
    state_dir.join("env")
}

/// Resolve the path to a project-scoped env file.
pub fn project_env_path(state_dir: &Path, project: &str) -> PathBuf {
    state_dir.join(format!("env.{project}"))
}

/// Parse a dotenv-style file into ordered key-value pairs.
/// Returns an empty map if the file doesn't exist.
pub fn read_env_file(path: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e),
    };
    Ok(parse_env(&content))
}

/// Parse dotenv content string into key-value pairs.
fn parse_env(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim().to_string();
            let value = trimmed[eq_pos + 1..].to_string();
            if !key.is_empty() {
                map.insert(key, value);
            }
        }
    }
    map
}

/// Write a BTreeMap back to a dotenv-style file.
/// Creates parent directories if needed. Removes the file if the map is empty.
pub fn write_env_file(path: &Path, vars: &BTreeMap<String, String>) -> std::io::Result<()> {
    if vars.is_empty() {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content: String = vars
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, content + "\n")
    }
}

/// Load merged environment: global vars first, then project overrides.
/// Returns a Vec of (key, value) pairs ready for env injection.
pub fn load_merged_env(state_dir: &Path, namespace: &str) -> Vec<(String, String)> {
    let mut merged = BTreeMap::new();

    // Global vars (base layer)
    if let Ok(global) = read_env_file(&global_env_path(state_dir)) {
        merged.extend(global);
    }

    // Project vars (override layer)
    if !namespace.is_empty() {
        if let Ok(project) = read_env_file(&project_env_path(state_dir, namespace)) {
            merged.extend(project);
        }
    }

    merged.into_iter().collect()
}

#[cfg(test)]
#[path = "env_tests.rs"]
mod tests;
