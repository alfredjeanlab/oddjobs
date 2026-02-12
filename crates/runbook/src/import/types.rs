// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Import/const type definitions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A const definition in a library runbook.
///
/// ```hcl
/// const "prefix" {}                    # required, no default
/// const "check" { default = "true" }   # optional, has default
/// ```
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct ConstDef {
    #[serde(default)]
    pub default: Option<String>,
}

/// A const value provided at an import site.
///
/// ```hcl
/// const "prefix" { value = "oj" }
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ImportConst {
    pub value: String,
}

/// An import declaration in a user runbook.
///
/// ```hcl
/// import "oj/wok" {}
/// import "oj/wok" {
///   alias = "wok"
///   const "prefix" { value = "oj" }
/// }
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ImportDef {
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default, rename = "const")]
    pub consts: HashMap<String, ImportConst>,
}

impl ImportDef {
    /// Flatten const values into a simple string map.
    pub fn const_values(&self) -> HashMap<String, String> {
        self.consts.iter().map(|(k, v)| (k.clone(), v.value.clone())).collect()
    }
}

/// Warning from import resolution.
#[derive(Debug, Clone)]
pub enum ImportWarning {
    /// Local entity overrides an imported entity with the same name.
    LocalOverride { entity_type: &'static str, name: String, source: String },
    /// Unknown const provided at import site.
    UnknownConst { source: String, name: String },
}

impl std::fmt::Display for ImportWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportWarning::LocalOverride { entity_type, name, source } => {
                write!(f, "local {} '{}' overrides imported from '{}'", entity_type, name, source)
            }
            ImportWarning::UnknownConst { source, name } => {
                write!(f, "unknown const '{}' for import '{}'", name, source)
            }
        }
    }
}
