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

use crate::find::extract_file_comment;
use crate::parser::{Format, ParseError, Runbook};
use crate::template::escape_for_shell;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

// =============================================================================
// Types
// =============================================================================

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
        self.consts
            .iter()
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect()
    }
}

/// Warning from import resolution.
#[derive(Debug, Clone)]
pub enum ImportWarning {
    /// Local entity overrides an imported entity with the same name.
    LocalOverride {
        entity_type: &'static str,
        name: String,
        source: String,
    },
    /// Unknown const provided at import site.
    UnknownConst { source: String, name: String },
}

impl std::fmt::Display for ImportWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportWarning::LocalOverride {
                entity_type,
                name,
                source,
            } => write!(
                f,
                "local {} '{}' overrides imported from '{}'",
                entity_type, name, source
            ),
            ImportWarning::UnknownConst { source, name } => {
                write!(f, "unknown const '{}' for import '{}'", name, source)
            }
        }
    }
}

// =============================================================================
// Const interpolation
// =============================================================================

/// Regex for `%{ if const.name == "x" }` directives.
///
/// Supports:
/// - `%{ if const.name == "x" }` — equality
/// - `%{ if const.name != "x" }` — inequality
#[allow(clippy::expect_used)]
static IF_DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"%\{~?\s*if\s+const\.([a-zA-Z_][a-zA-Z0-9_]*)\s*(==|!=)\s*"([^"]*)"\s*~?\}"#)
        .expect("constant regex pattern is valid")
});

/// Regex for `%{ else }` directives.
#[allow(clippy::expect_used)]
static ELSE_DIRECTIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"%\{~?\s*else\s*~?\}").expect("constant regex pattern is valid"));

/// Regex for `%{ endif }` directives.
#[allow(clippy::expect_used)]
static ENDIF_DIRECTIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"%\{~?\s*endif\s*~?\}").expect("constant regex pattern is valid"));

/// Process `%{ if const.name == "x" }` / `%{ else }` / `%{ endif }` directives.
///
/// Evaluates conditionals based on const values. Supports:
/// - Comparison: `%{ if const.name == "value" }` / `%{ if const.name != "value" }`
/// - `%{ else }` branches and nesting
fn process_const_directives(
    content: &str,
    values: &HashMap<String, String>,
) -> Result<String, String> {
    let mut kept_lines: Vec<&str> = Vec::new();
    // Stack of (active, else_seen) — active means we're emitting lines
    let mut stack: Vec<(bool, bool)> = Vec::new();

    for line in content.split('\n') {
        if let Some(caps) = IF_DIRECTIVE.captures(line) {
            let name = &caps[1];
            let op = &caps[2];
            let literal = &caps[3];
            let value = values.get(name).map(|v| v.as_str()).unwrap_or("");
            let condition = {
                let matches = value == literal;
                if op == "==" {
                    matches
                } else {
                    !matches
                }
            };
            // Only active if parent is active (or we're at top level)
            let parent_active = stack.last().is_none_or(|&(a, _)| a);
            stack.push((parent_active && condition, false));
            continue;
        }

        if ELSE_DIRECTIVE.is_match(line) {
            let len = stack.len();
            if len == 0 {
                return Err("else without matching if".to_string());
            }
            if stack[len - 1].1 {
                return Err("duplicate else".to_string());
            }
            stack[len - 1].1 = true;
            let parent_active = if len > 1 { stack[len - 2].0 } else { true };
            // Flip: if parent is active, toggle current; if parent inactive, stay inactive
            stack[len - 1].0 = parent_active && !stack[len - 1].0;
            continue;
        }

        if ENDIF_DIRECTIVE.is_match(line) {
            if stack.is_empty() {
                return Err("endif without matching if".to_string());
            }
            stack.pop();
            continue;
        }

        // Keep line if all levels are active
        let active = stack.last().is_none_or(|&(a, _)| a);
        if active {
            kept_lines.push(line);
        }
    }

    if !stack.is_empty() {
        return Err("unclosed if directive".to_string());
    }

    Ok(kept_lines.join("\n"))
}

/// Regex for `${raw(const.name)}` patterns.
#[allow(clippy::expect_used)]
static RAW_CONST_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{raw\(const\.([a-zA-Z_][a-zA-Z0-9_]*)\)\}")
        .expect("constant regex pattern is valid")
});

/// Regex for `${const.name}` patterns.
#[allow(clippy::expect_used)]
static CONST_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{const\.([a-zA-Z_][a-zA-Z0-9_]*)\}").expect("constant regex pattern is valid")
});

/// Strip `%{ if/else/endif }` const directives from library content.
///
/// Evaluates all conditionals with empty values, effectively removing all
/// directive lines and any content inside `%{ if const.X != "" }` blocks.
/// Used for lightweight parsing (e.g. extracting entity names) when const
/// values are not yet known.
pub fn strip_const_directives(content: &str) -> Result<String, String> {
    process_const_directives(content, &HashMap::new())
}

/// Interpolate const values into library content.
///
/// 1. Evaluate `%{ if/else/endif }` directives — strip or keep text blocks
/// 2. Substitute `${raw(const.name)}` and `${const.name}` in remaining text
pub fn interpolate_consts(
    content: &str,
    values: &HashMap<String, String>,
) -> Result<String, String> {
    let content = process_const_directives(content, values)?;

    // Replace ${raw(const.name)} with raw values
    let result = RAW_CONST_PATTERN
        .replace_all(&content, |caps: &regex::Captures| {
            let name = &caps[1];
            values
                .get(name)
                .cloned()
                .unwrap_or_else(|| caps[0].to_string())
        })
        .to_string();

    // Replace ${const.name} with shell-escaped values
    Ok(CONST_PATTERN
        .replace_all(&result, |caps: &regex::Captures| {
            let name = &caps[1];
            match values.get(name) {
                Some(val) => escape_for_shell(val),
                None => caps[0].to_string(),
            }
        })
        .to_string())
}

/// Validate const values against const definitions.
///
/// Returns the resolved values map (with defaults applied) and any warnings.
pub fn validate_consts(
    defs: &HashMap<String, ConstDef>,
    provided: &HashMap<String, String>,
    source: &str,
) -> Result<(HashMap<String, String>, Vec<ImportWarning>), ParseError> {
    let mut values = HashMap::new();
    let mut warnings = Vec::new();

    // Check each defined const
    for (name, def) in defs {
        match provided.get(name) {
            Some(val) => {
                values.insert(name.clone(), val.clone());
            }
            None => match &def.default {
                Some(default) => {
                    values.insert(name.clone(), default.clone());
                }
                None => {
                    return Err(ParseError::InvalidFormat {
                        location: format!("import \"{}\"", source),
                        message: format!(
                            "missing required const '{}'; add const \"{}\" {{ value = \"...\" }}",
                            name, name
                        ),
                    });
                }
            },
        }
    }

    // Warn on unknown consts
    for name in provided.keys() {
        if !defs.contains_key(name) {
            warnings.push(ImportWarning::UnknownConst {
                source: source.to_string(),
                name: name.clone(),
            });
        }
    }

    Ok((values, warnings))
}

// =============================================================================
// Library resolution
// =============================================================================

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

// =============================================================================
// Merge logic
// =============================================================================

/// Merge an imported runbook into the target runbook.
///
/// If `alias` is provided, all entity names in `source` are prefixed with `alias:`.
/// Internal cross-references within the imported runbook are also updated.
///
/// Conflict handling:
/// - Local entity vs import with same name → local wins (warning)
/// - Import A vs Import B with same name → error
pub fn merge_runbook(
    target: &mut Runbook,
    mut source: Runbook,
    alias: Option<&str>,
    import_source: &str,
) -> Result<Vec<ImportWarning>, ParseError> {
    let mut warnings = Vec::new();

    if let Some(prefix) = alias {
        prefix_names(&mut source, prefix);
    }

    // Merge each entity type
    merge_map(
        &mut target.commands,
        source.commands,
        "command",
        import_source,
        &mut warnings,
    )?;
    merge_map(
        &mut target.jobs,
        source.jobs,
        "job",
        import_source,
        &mut warnings,
    )?;
    merge_map(
        &mut target.agents,
        source.agents,
        "agent",
        import_source,
        &mut warnings,
    )?;
    merge_map(
        &mut target.queues,
        source.queues,
        "queue",
        import_source,
        &mut warnings,
    )?;
    merge_map(
        &mut target.workers,
        source.workers,
        "worker",
        import_source,
        &mut warnings,
    )?;
    merge_map(
        &mut target.crons,
        source.crons,
        "cron",
        import_source,
        &mut warnings,
    )?;

    Ok(warnings)
}

/// Merge a source map into a target map, with conflict handling.
fn merge_map<V>(
    target: &mut HashMap<String, V>,
    source: HashMap<String, V>,
    entity_type: &'static str,
    import_source: &str,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), ParseError> {
    for (name, value) in source {
        use std::collections::hash_map::Entry;
        match target.entry(name) {
            Entry::Occupied(e) => {
                // Local wins — emit warning
                warnings.push(ImportWarning::LocalOverride {
                    entity_type,
                    name: e.key().clone(),
                    source: import_source.to_string(),
                });
            }
            Entry::Vacant(e) => {
                e.insert(value);
            }
        }
    }
    Ok(())
}

/// Prefix all entity names and update internal cross-references.
fn prefix_names(runbook: &mut Runbook, prefix: &str) {
    // Collect old→new name mappings for each entity type
    let cmd_renames: HashMap<String, String> = runbook
        .commands
        .keys()
        .map(|k| (k.clone(), format!("{}:{}", prefix, k)))
        .collect();
    let job_renames: HashMap<String, String> = runbook
        .jobs
        .keys()
        .map(|k| (k.clone(), format!("{}:{}", prefix, k)))
        .collect();
    let agent_renames: HashMap<String, String> = runbook
        .agents
        .keys()
        .map(|k| (k.clone(), format!("{}:{}", prefix, k)))
        .collect();
    let queue_renames: HashMap<String, String> = runbook
        .queues
        .keys()
        .map(|k| (k.clone(), format!("{}:{}", prefix, k)))
        .collect();
    let worker_renames: HashMap<String, String> = runbook
        .workers
        .keys()
        .map(|k| (k.clone(), format!("{}:{}", prefix, k)))
        .collect();
    let cron_renames: HashMap<String, String> = runbook
        .crons
        .keys()
        .map(|k| (k.clone(), format!("{}:{}", prefix, k)))
        .collect();

    // Rename entity map keys
    runbook.commands = rename_keys(std::mem::take(&mut runbook.commands), &cmd_renames);
    runbook.jobs = rename_keys(std::mem::take(&mut runbook.jobs), &job_renames);
    runbook.agents = rename_keys(std::mem::take(&mut runbook.agents), &agent_renames);
    runbook.queues = rename_keys(std::mem::take(&mut runbook.queues), &queue_renames);
    runbook.workers = rename_keys(std::mem::take(&mut runbook.workers), &worker_renames);
    runbook.crons = rename_keys(std::mem::take(&mut runbook.crons), &cron_renames);

    // Update .name fields
    for (key, cmd) in &mut runbook.commands {
        cmd.name = key.clone();
    }
    for (key, job) in &mut runbook.jobs {
        job.kind = key.clone();
    }
    for (key, agent) in &mut runbook.agents {
        agent.name = key.clone();
    }
    for (key, queue) in &mut runbook.queues {
        queue.name = key.clone();
    }
    for (key, worker) in &mut runbook.workers {
        worker.name = key.clone();
    }
    for (key, cron) in &mut runbook.crons {
        cron.name = key.clone();
    }

    // Update internal cross-references
    for worker in runbook.workers.values_mut() {
        if let Some(new) = queue_renames.get(&worker.source.queue) {
            worker.source.queue = new.clone();
        }
        if let Some(new) = job_renames.get(&worker.handler.job) {
            worker.handler.job = new.clone();
        }
    }

    for cron in runbook.crons.values_mut() {
        rename_run_directive(&mut cron.run, &job_renames, &agent_renames);
    }

    for cmd in runbook.commands.values_mut() {
        rename_run_directive(&mut cmd.run, &job_renames, &agent_renames);
    }

    for job in runbook.jobs.values_mut() {
        for step in &mut job.steps {
            rename_run_directive(&mut step.run, &job_renames, &agent_renames);
        }
    }
}

fn rename_keys<V>(
    map: HashMap<String, V>,
    renames: &HashMap<String, String>,
) -> HashMap<String, V> {
    map.into_iter()
        .map(|(k, v)| {
            let new_key = renames.get(&k).cloned().unwrap_or(k);
            (new_key, v)
        })
        .collect()
}

fn rename_run_directive(
    directive: &mut crate::RunDirective,
    job_renames: &HashMap<String, String>,
    agent_renames: &HashMap<String, String>,
) {
    match directive {
        crate::RunDirective::Job { job } => {
            if let Some(new) = job_renames.get(job.as_str()) {
                *job = new.clone();
            }
        }
        crate::RunDirective::Agent { agent, .. } => {
            if let Some(new) = agent_renames.get(agent.as_str()) {
                *agent = new.clone();
            }
        }
        crate::RunDirective::Shell(_) => {}
    }
}

// =============================================================================
// Top-level pipeline
// =============================================================================

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
#[path = "import_tests.rs"]
mod tests;
