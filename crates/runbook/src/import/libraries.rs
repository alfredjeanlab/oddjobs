// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Built-in library resolution.

use crate::find::extract_file_comment;
use crate::parser::ParseError;

/// A built-in library: a named collection of HCL files.
struct BuiltinLibrary {
    source: &'static str,
    files: &'static [(&'static str, &'static str)],
}

// Registry of all built-in libraries (auto-generated from `library/` directory).
include!(concat!(env!("OUT_DIR"), "/builtin_libraries.rs"));

/// Metadata about a built-in library.
#[derive(Debug, Clone)]
pub struct LibraryInfo {
    pub source: &'static str,
    pub files: &'static [(&'static str, &'static str)],
    pub description: String,
}

/// Return metadata for all built-in libraries.
pub fn available_libraries() -> Vec<LibraryInfo> {
    BUILTIN_LIBRARIES
        .iter()
        .map(|lib| {
            let description = lib
                .files
                .first()
                .and_then(|(_, content)| extract_file_comment(content))
                .map(|c| c.short)
                .unwrap_or_default();
            LibraryInfo {
                source: lib.source,
                files: lib.files,
                description,
            }
        })
        .collect()
}

/// Resolve a library source path to its HCL file list.
pub fn resolve_library(
    source: &str,
) -> Result<&'static [(&'static str, &'static str)], ParseError> {
    for lib in BUILTIN_LIBRARIES {
        if lib.source == source {
            return Ok(lib.files);
        }
    }
    let available: Vec<&str> = BUILTIN_LIBRARIES.iter().map(|l| l.source).collect();
    Err(ParseError::InvalidFormat {
        location: format!("import \"{}\"", source),
        message: format!(
            "unknown library '{}'; available libraries: {}",
            source,
            available.join(", ")
        ),
    })
}
