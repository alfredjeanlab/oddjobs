// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Runbook import system
//!
//! Handles `import` and `const` blocks in HCL runbooks:
//!
//! ```hcl
//! import "oj/wok" { const = { prefix = "oj" } }
//! import "oj/merge" "merge" {}
//! ```

use crate::find::extract_file_comment;
use crate::parser::{Format, ParseError, Runbook};
use crate::template::escape_for_shell;
use regex::Regex;
use serde::Deserialize;
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
#[derive(Debug, Clone)]
pub struct ConstDef {
    pub name: String,
    pub default: Option<String>,
}

/// An import declaration in a user runbook.
///
/// ```hcl
/// import "oj/wok" {}
/// import "oj/wok" "wok" { const = { prefix = "oj" } }
/// ```
#[derive(Debug, Clone)]
pub struct ImportDef {
    pub source: String,
    pub alias: Option<String>,
    pub consts: HashMap<String, String>,
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
// Serde helpers for parsing block bodies
// =============================================================================

#[derive(Deserialize, Default)]
struct ImportBody {
    #[serde(default, rename = "const")]
    consts: HashMap<String, String>,
}

#[derive(Deserialize, Default)]
struct ConstBody {
    #[serde(default)]
    default: Option<String>,
}

// =============================================================================
// Block extraction
// =============================================================================

/// Extract quoted string labels from a block header line.
///
/// Given `import "oj/wok" "wok" {`, returns `["oj/wok", "wok"]`.
fn extract_labels(line: &str, block_id: &str) -> Vec<String> {
    let rest = line.trim().strip_prefix(block_id).unwrap_or("");
    let mut labels = Vec::new();
    let mut chars = rest.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c == '"' {
            chars.next(); // consume opening quote
            let mut label = String::new();
            loop {
                match chars.next() {
                    Some('"') | None => break,
                    Some('\\') => {
                        if let Some(escaped) = chars.next() {
                            label.push(escaped);
                        }
                    }
                    Some(c) => label.push(c),
                }
            }
            labels.push(label);
        } else if c == '{' {
            break;
        } else {
            chars.next();
        }
    }

    labels
}

/// Find the position of the matching closing brace, handling nested braces and strings.
///
/// `start` should be the position of the opening `{`.
/// Returns the position of the matching `}`.
fn find_closing_brace(content: &str, start: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut depth: i32 = 0;
    let mut i = start;
    let mut in_string = false;
    let mut escape_next = false;

    while i < bytes.len() {
        if escape_next {
            escape_next = false;
            i += 1;
            continue;
        }
        match bytes[i] {
            b'\\' if in_string => escape_next = true,
            b'"' => in_string = !in_string,
            b'{' if !in_string => depth += 1,
            b'}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Result of extracting import/const blocks from HCL content.
pub struct ExtractResult {
    pub imports: Vec<ImportDef>,
    pub consts: Vec<ConstDef>,
    pub remaining: String,
}

/// Extract `import` and `const` blocks from HCL content.
///
/// Returns the extracted definitions and the remaining HCL content
/// with those blocks removed (safe for serde parsing).
pub fn extract_blocks(content: &str) -> Result<ExtractResult, ParseError> {
    let mut imports = Vec::new();
    let mut consts = Vec::new();

    // Track byte ranges to remove
    let mut remove_ranges: Vec<(usize, usize)> = Vec::new();

    let mut search_from = 0;
    while search_from < content.len() {
        // Find next import or const block
        let rest = &content[search_from..];

        // Look for block start patterns
        let import_pos = find_block_start(rest, "import");
        let const_pos = find_block_start(rest, "const");

        let (block_id, offset) = match (import_pos, const_pos) {
            (Some(ip), Some(cp)) => {
                if ip <= cp {
                    ("import", ip)
                } else {
                    ("const", cp)
                }
            }
            (Some(ip), None) => ("import", ip),
            (None, Some(cp)) => ("const", cp),
            (None, None) => break,
        };

        let abs_offset = search_from + offset;
        let line_content = &content[abs_offset..];

        // Extract labels
        let labels = extract_labels(line_content, block_id);

        // Find opening brace
        let brace_start = match content[abs_offset..].find('{') {
            Some(pos) => abs_offset + pos,
            None => {
                return Err(ParseError::InvalidFormat {
                    location: format!("{} block", block_id),
                    message: "missing opening brace".to_string(),
                });
            }
        };

        // Find matching closing brace
        let brace_end = match find_closing_brace(content, brace_start) {
            Some(pos) => pos,
            None => {
                return Err(ParseError::InvalidFormat {
                    location: format!("{} block", block_id),
                    message: "missing closing brace".to_string(),
                });
            }
        };

        // Extract body content (between braces, exclusive)
        let body = &content[brace_start + 1..brace_end];

        match block_id {
            "import" => {
                let source = labels
                    .first()
                    .cloned()
                    .ok_or_else(|| ParseError::InvalidFormat {
                        location: "import block".to_string(),
                        message: "import requires a source path label".to_string(),
                    })?;
                let alias = labels.get(1).cloned();
                let import_body: ImportBody = if body.trim().is_empty() {
                    ImportBody::default()
                } else {
                    hcl::from_str(body).map_err(|e| ParseError::InvalidFormat {
                        location: format!("import \"{}\"", source),
                        message: format!("invalid import body: {}", e),
                    })?
                };
                imports.push(ImportDef {
                    source,
                    alias,
                    consts: import_body.consts,
                });
            }
            "const" => {
                let name = labels
                    .first()
                    .cloned()
                    .ok_or_else(|| ParseError::InvalidFormat {
                        location: "const block".to_string(),
                        message: "const requires a name label".to_string(),
                    })?;
                let const_body: ConstBody = if body.trim().is_empty() {
                    ConstBody::default()
                } else {
                    hcl::from_str(body).map_err(|e| ParseError::InvalidFormat {
                        location: format!("const \"{}\"", name),
                        message: format!("invalid const body: {}", e),
                    })?
                };
                consts.push(ConstDef {
                    name,
                    default: const_body.default,
                });
            }
            _ => unreachable!(),
        }

        // Record range to remove (from block start to after closing brace + newline)
        let end = if brace_end + 1 < content.len() && content.as_bytes()[brace_end + 1] == b'\n' {
            brace_end + 2
        } else {
            brace_end + 1
        };
        remove_ranges.push((abs_offset, end));
        search_from = end;
    }

    // Build remaining content by removing the extracted block ranges
    let mut remaining = String::with_capacity(content.len());
    let mut last_end = 0;
    for (start, end) in &remove_ranges {
        remaining.push_str(&content[last_end..*start]);
        last_end = *end;
    }
    remaining.push_str(&content[last_end..]);

    Ok(ExtractResult {
        imports,
        consts,
        remaining,
    })
}

/// Find the byte offset of the next block start for the given identifier.
///
/// Looks for lines starting with `{id} "` (with optional leading whitespace).
fn find_block_start(content: &str, id: &str) -> Option<usize> {
    let pattern = format!("{} \"", id);
    for (line_offset, line) in content.split('\n').scan(0usize, |offset, line| {
        let start = *offset;
        *offset += line.len() + 1; // +1 for newline
        Some((start, line))
    }) {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&pattern) {
            // Return offset of the trimmed start
            let leading_ws = line.len() - trimmed.len();
            return Some(line_offset + leading_ws);
        }
    }
    None
}

// =============================================================================
// Const interpolation
// =============================================================================

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

/// Interpolate const values into library content.
///
/// - `${const.name}` → shell-escaped value (safe for data)
/// - `${raw(const.name)}` → raw value (for command templates)
pub fn interpolate_consts(content: &str, values: &HashMap<String, String>) -> String {
    // First, replace ${raw(const.name)} with raw values
    let result = RAW_CONST_PATTERN
        .replace_all(content, |caps: &regex::Captures| {
            let name = &caps[1];
            values
                .get(name)
                .cloned()
                .unwrap_or_else(|| caps[0].to_string())
        })
        .to_string();

    // Then, replace ${const.name} with shell-escaped values
    CONST_PATTERN
        .replace_all(&result, |caps: &regex::Captures| {
            let name = &caps[1];
            match values.get(name) {
                Some(val) => escape_for_shell(val),
                None => caps[0].to_string(),
            }
        })
        .to_string()
}

/// Validate const values against const definitions.
///
/// Returns the resolved values map (with defaults applied) and any warnings.
pub fn validate_consts(
    defs: &[ConstDef],
    provided: &HashMap<String, String>,
    source: &str,
) -> Result<(HashMap<String, String>, Vec<ImportWarning>), ParseError> {
    let mut values = HashMap::new();
    let mut warnings = Vec::new();

    // Check each defined const
    for def in defs {
        match provided.get(&def.name) {
            Some(val) => {
                values.insert(def.name.clone(), val.clone());
            }
            None => match &def.default {
                Some(default) => {
                    values.insert(def.name.clone(), default.clone());
                }
                None => {
                    return Err(ParseError::InvalidFormat {
                        location: format!("import \"{}\"", source),
                        message: format!(
                            "missing required const '{}'; add const = {{ {} = \"...\" }}",
                            def.name, def.name
                        ),
                    });
                }
            },
        }
    }

    // Warn on unknown consts
    let known: std::collections::HashSet<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    for name in provided.keys() {
        if !known.contains(name.as_str()) {
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

/// Built-in library: oj/wok
const LIBRARY_WOK: &str = include_str!("library/wok.hcl");

/// Built-in library: oj/merge
const LIBRARY_MERGE: &str = include_str!("library/merge.hcl");

/// Metadata about a built-in library.
#[derive(Debug, Clone)]
pub struct LibraryInfo {
    pub source: &'static str,
    pub content: &'static str,
    pub description: String,
}

/// Return metadata for all built-in libraries.
pub fn available_libraries() -> Vec<LibraryInfo> {
    static SOURCES: &[(&str, &str)] = &[("oj/merge", LIBRARY_MERGE), ("oj/wok", LIBRARY_WOK)];
    SOURCES
        .iter()
        .map(|(source, content)| {
            let description = extract_file_comment(content)
                .map(|c| c.short)
                .unwrap_or_default();
            LibraryInfo {
                source,
                content,
                description,
            }
        })
        .collect()
}

/// Resolve a library source path to its HCL content.
///
/// Built-in libraries:
/// - `oj/wok` — Wok-based issue queues (fix, chore)
/// - `oj/merge` — Local merge queue with conflict resolution
pub fn resolve_library(source: &str) -> Result<&'static str, ParseError> {
    match source {
        "oj/wok" => Ok(LIBRARY_WOK),
        "oj/merge" => Ok(LIBRARY_MERGE),
        _ => Err(ParseError::InvalidFormat {
            location: format!("import \"{}\"", source),
            message: format!(
                "unknown library '{}'; available libraries: oj/wok, oj/merge",
                source
            ),
        }),
    }
}

// =============================================================================
// Merge logic
// =============================================================================

/// Merge an imported runbook into the target runbook.
///
/// If `alias` is provided, all entity names in `source` are prefixed with `alias.`.
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
        .map(|k| (k.clone(), format!("{}.{}", prefix, k)))
        .collect();
    let job_renames: HashMap<String, String> = runbook
        .jobs
        .keys()
        .map(|k| (k.clone(), format!("{}.{}", prefix, k)))
        .collect();
    let agent_renames: HashMap<String, String> = runbook
        .agents
        .keys()
        .map(|k| (k.clone(), format!("{}.{}", prefix, k)))
        .collect();
    let queue_renames: HashMap<String, String> = runbook
        .queues
        .keys()
        .map(|k| (k.clone(), format!("{}.{}", prefix, k)))
        .collect();
    let worker_renames: HashMap<String, String> = runbook
        .workers
        .keys()
        .map(|k| (k.clone(), format!("{}.{}", prefix, k)))
        .collect();
    let cron_renames: HashMap<String, String> = runbook
        .crons
        .keys()
        .map(|k| (k.clone(), format!("{}.{}", prefix, k)))
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
/// 1. Extracts `import` and `const` blocks from the content
/// 2. Parses the remaining content as a normal runbook
/// 3. For each import, loads the library, validates consts, interpolates, parses, and merges
/// 4. Validates cross-references on the merged result
///
/// Returns the merged runbook and any warnings.
pub fn parse_with_imports(
    content: &str,
    format: Format,
) -> Result<(Runbook, Vec<ImportWarning>), ParseError> {
    let extracted = extract_blocks(content)?;

    if extracted.imports.is_empty() && extracted.consts.is_empty() {
        // No imports/consts — parse normally
        let runbook = crate::parser::parse_runbook_with_format(content, format)?;
        return Ok((runbook, Vec::new()));
    }

    // Parse the remaining content without cross-ref validation
    let mut runbook = crate::parser::parse_runbook_no_xref(&extracted.remaining, format)?;
    let mut all_warnings = Vec::new();

    // Resolve each import
    for import_def in &extracted.imports {
        let library_content = resolve_library(&import_def.source)?;

        // Extract const definitions from library
        let lib_extracted = extract_blocks(library_content)?;

        // Validate and resolve const values
        let (const_values, const_warnings) = validate_consts(
            &lib_extracted.consts,
            &import_def.consts,
            &import_def.source,
        )?;
        all_warnings.extend(const_warnings);

        // Interpolate consts into library content
        let interpolated = interpolate_consts(&lib_extracted.remaining, &const_values);

        // Parse the interpolated library content
        let lib_runbook = crate::parser::parse_runbook_with_format(&interpolated, Format::Hcl)?;

        // Merge into the main runbook
        let merge_warnings = merge_runbook(
            &mut runbook,
            lib_runbook,
            import_def.alias.as_deref(),
            &import_def.source,
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
