// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Variable namespacing helpers

use std::collections::HashMap;

/// Known variable scope prefixes.
const SCOPE_PREFIXES: &[&str] = &["var.", "invoke.", "workspace.", "local.", "args."];

/// Returns true if `key` already has a recognized scope prefix.
fn has_scope_prefix(key: &str) -> bool {
    SCOPE_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Namespace bare keys under the `var.` prefix.
///
/// Keys that already carry a scope prefix (`var.`, `invoke.`, `workspace.`,
/// `local.`, `args.`, `item.`) are kept as-is to avoid double-prefixing.
pub fn namespace_vars(input: &HashMap<String, String>) -> HashMap<String, String> {
    input
        .iter()
        .map(|(k, v)| {
            if has_scope_prefix(k) {
                (k.clone(), v.clone())
            } else {
                (format!("var.{}", k), v.clone())
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "vars_tests.rs"]
mod tests;
