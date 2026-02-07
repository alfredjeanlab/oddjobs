// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Built-in and external library resolution.

use crate::find::extract_file_comment;
use crate::parser::ParseError;
use std::path::{Path, PathBuf};

/// A library's file contents: Vec of (filename, content) pairs.
pub type LibraryFiles = Vec<(String, String)>;

/// A built-in library: a named collection of HCL files.
struct BuiltinLibrary {
    source: &'static str,
    files: &'static [(&'static str, &'static str)],
}

// Registry of all built-in libraries (auto-generated from `library/` directory).
include!(concat!(env!("OUT_DIR"), "/builtin_libraries.rs"));

/// Read sorted `.hcl` files from a directory, returning (filename, content) pairs.
fn read_library_dir(dir: &Path) -> Result<LibraryFiles, std::io::Error> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "hcl") {
            let filename = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let content = std::fs::read_to_string(&path)?;
            files.push((filename, content));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// Metadata about a library.
#[derive(Debug, Clone)]
pub struct LibraryInfo {
    pub source: String,
    pub files: LibraryFiles,
    pub description: String,
}

/// Return metadata for all available libraries.
///
/// Scans `library_dirs` for subdirectories (external libraries), then appends
/// built-in libraries. Earlier entries shadow later ones by source name.
pub fn available_libraries(library_dirs: &[PathBuf]) -> Vec<LibraryInfo> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    // External libraries from library_dirs
    for dir in library_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut subdirs: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
            subdirs.sort_by_key(|e| e.file_name());
            for entry in subdirs {
                let source = entry.file_name().to_string_lossy().to_string();
                if seen.contains(&source) {
                    continue;
                }
                let files = match read_library_dir(&entry.path()) {
                    Ok(f) if !f.is_empty() => f,
                    _ => continue,
                };
                let description = files
                    .first()
                    .and_then(|(_, content)| extract_file_comment(content))
                    .map(|c| c.short)
                    .unwrap_or_default();
                seen.insert(source.clone());
                result.push(LibraryInfo {
                    source,
                    files,
                    description,
                });
            }
        }
    }

    // Built-in libraries
    for lib in BUILTIN_LIBRARIES {
        let source = lib.source.to_string();
        if seen.contains(&source) {
            continue;
        }
        let description = lib
            .files
            .first()
            .and_then(|(_, content)| extract_file_comment(content))
            .map(|c| c.short)
            .unwrap_or_default();
        seen.insert(source.clone());
        result.push(LibraryInfo {
            source,
            files: lib
                .files
                .iter()
                .map(|(f, c)| (f.to_string(), c.to_string()))
                .collect(),
            description,
        });
    }

    result
}

/// Resolve a library source path to its HCL file list.
///
/// Checks `library_dirs` in order for a `<dir>/<source>/` subdirectory,
/// then falls back to built-in libraries.
pub fn resolve_library(source: &str, library_dirs: &[PathBuf]) -> Result<LibraryFiles, ParseError> {
    // Check external library dirs
    for dir in library_dirs {
        let lib_dir = dir.join(source);
        if lib_dir.is_dir() {
            return read_library_dir(&lib_dir).map_err(|e| ParseError::InvalidFormat {
                location: format!("import \"{}\"", source),
                message: format!("failed to read library directory: {}", e),
            });
        }
    }

    // Fall back to built-in
    for lib in BUILTIN_LIBRARIES {
        if lib.source == source {
            return Ok(lib
                .files
                .iter()
                .map(|(f, c)| (f.to_string(), c.to_string()))
                .collect());
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
