// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Project namespace resolution and the [`Namespace`] newtype.

use std::path::Path;

/// A project namespace identifier.
///
/// Wraps a `String` to distinguish namespace values from other string fields
/// (e.g. queue names, worker names) at the type level. An empty `Namespace`
/// represents the absence of a project scope (backward compatibility with
/// old events that predate namespace support).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Namespace(String);

impl Namespace {
    /// Create a new `Namespace` from any string-like value.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Convert to `Option<&str>`, mapping empty to `None`.
    pub fn to_option(&self) -> Option<&str> {
        namespace_to_option(&self.0)
    }

    /// Consume the newtype, returning the inner `String`.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::ops::Deref for Namespace {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<String> for Namespace {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Namespace {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for Namespace {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Convert a namespace string to `Option<&str>`, mapping empty to `None`.
///
/// Many APIs accept an empty string as "no namespace". This helper centralises
/// the conversion so callers don't repeat the `is_empty` / `Some` / `None`
/// boilerplate.
pub fn namespace_to_option(ns: &str) -> Option<&str> {
    if ns.is_empty() {
        None
    } else {
        Some(ns)
    }
}

/// Build a namespace-scoped key from namespace and name.
///
/// When namespace is empty (backward compat with old events), returns the bare name.
/// Otherwise returns `"{namespace}/{name}"`.
pub fn scoped_name(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", namespace, name)
    }
}

/// Parse a namespace-scoped key into `(namespace, name)`.
///
/// Returns `("", key)` when no slash is present.
pub fn split_scoped_name(scoped: &str) -> (&str, &str) {
    match scoped.split_once('/') {
        Some((ns, name)) => (ns, name),
        None => ("", scoped),
    }
}

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
