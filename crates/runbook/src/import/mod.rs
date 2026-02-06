// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runbook import system
//!
//! Handles `import` and `const` blocks in HCL runbooks:
//!
//! ```hcl
//! import "oj/wok" { const "prefix" { value = "oj" } }
//! import "oj/git" { alias = "git" }
//! ```

mod consts;
mod libraries;
mod merge;
mod types;

pub use consts::{interpolate_consts, strip_const_directives, validate_consts};
pub use libraries::{available_libraries, resolve_library, LibraryInfo};
pub use merge::merge_runbook;
pub use types::{ConstDef, ImportConst, ImportDef, ImportWarning};

use crate::parser::{Format, ParseError, Runbook};
use consts::process_const_directives;
use std::collections::HashMap;

/// Parse an HCL runbook with import resolution.
///
/// 1. Parses the content (import/const blocks are regular serde fields)
/// 2. For each import, loads the library, validates consts, interpolates, parses, and merges
/// 3. Validates cross-references on the merged result
///
/// Returns the merged runbook and any warnings.
pub fn parse_with_imports(
    content: &str,
    format: Format,
) -> Result<(Runbook, Vec<ImportWarning>), ParseError> {
    // Parse full content — imports and consts are now regular Runbook fields
    let mut runbook = crate::parser::parse_runbook_no_xref(content, format)?;

    if runbook.imports.is_empty() {
        // No imports — validate cross-refs and return
        runbook.consts.clear();
        crate::parser::validate_cross_refs(&runbook)?;
        return Ok((runbook, Vec::new()));
    }

    // Take imports, clear metadata fields
    let imports = std::mem::take(&mut runbook.imports);
    runbook.consts.clear();

    let mut all_warnings = Vec::new();

    // Resolve each import
    for (source, import_def) in &imports {
        let library_files = resolve_library(source)?;

        // Collect const definitions from all files in the library
        let mut all_const_defs: HashMap<String, ConstDef> = HashMap::new();
        let empty_values = HashMap::new();
        for (filename, content) in library_files {
            // Strip directives before parsing to avoid shell validation errors
            // on template content (const defs are never inside conditional blocks)
            let stripped = process_const_directives(content, &empty_values).map_err(|msg| {
                ParseError::InvalidFormat {
                    location: format!("import \"{}/{}\"", source, filename),
                    message: msg,
                }
            })?;
            let file_meta = crate::parser::parse_runbook_no_xref(&stripped, Format::Hcl)?;
            for (name, def) in file_meta.consts {
                if let Some(existing) = all_const_defs.get(&name) {
                    if *existing != def {
                        return Err(ParseError::InvalidFormat {
                            location: format!("import \"{}\"", source),
                            message: format!(
                                "conflicting const '{}' in library file '{}'",
                                name, filename
                            ),
                        });
                    }
                } else {
                    all_const_defs.insert(name, def);
                }
            }
        }

        // Validate and resolve const values
        let (const_values, const_warnings) =
            validate_consts(&all_const_defs, &import_def.const_values(), source)?;
        all_warnings.extend(const_warnings);

        // Parse each file, interpolate consts, and merge into a single library runbook
        let mut lib_runbook = Runbook::default();
        for (filename, content) in library_files {
            let interpolated = interpolate_consts(content, &const_values).map_err(|msg| {
                ParseError::InvalidFormat {
                    location: format!("import \"{}/{}\"", source, filename),
                    message: msg,
                }
            })?;
            let mut file_runbook =
                crate::parser::parse_runbook_with_format(&interpolated, Format::Hcl)?;
            file_runbook.consts.clear();
            file_runbook.imports.clear();

            let file_source = format!("{}/{}", source, filename);
            merge_runbook(&mut lib_runbook, file_runbook, None, &file_source)?;
        }

        // Validate intra-library cross-references
        crate::parser::validate_cross_refs(&lib_runbook)?;

        // Merge into the main runbook
        let merge_warnings = merge_runbook(
            &mut runbook,
            lib_runbook,
            import_def.alias.as_deref(),
            source,
        )?;
        all_warnings.extend(merge_warnings);
    }

    // Validate cross-references on the merged result
    crate::parser::validate_cross_refs(&runbook)?;

    Ok((runbook, all_warnings))
}

#[cfg(test)]
#[path = "../import_tests.rs"]
mod tests;
