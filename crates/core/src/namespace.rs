// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Project namespace resolution.

use std::path::Path;

/// Resolve the project namespace from a project root path.
///
/// 1. Read `.oj/config.toml` and return `[project].name` if present
/// 2. Fall back to the basename of `project_root`
/// 3. Fall back to "default" if basename is empty (e.g. root path "/")
pub fn resolve_namespace(project_root: &Path) -> String {
    if let Some(name) = read_config_name(project_root) {
        return name;
    }
    project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}

fn read_config_name(project_root: &Path) -> Option<String> {
    let config_path = project_root.join(".oj/config.toml");
    let content = std::fs::read_to_string(config_path).ok()?;
    let table: toml::Table = content.parse().ok()?;
    table
        .get("project")?
        .as_table()?
        .get("name")?
        .as_str()
        .map(String::from)
}

#[cfg(test)]
#[path = "namespace_tests.rs"]
mod tests;
