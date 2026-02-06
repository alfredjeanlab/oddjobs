// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;
use std::collections::HashMap;

// =============================================================================
// extract_labels tests
// =============================================================================

#[test]
fn extract_labels_single() {
    let labels = extract_labels(r#"import "oj/wok" {}"#, "import");
    assert_eq!(labels, vec!["oj/wok"]);
}

#[test]
fn extract_labels_two() {
    let labels = extract_labels(r#"import "oj/wok" "wok" {}"#, "import");
    assert_eq!(labels, vec!["oj/wok", "wok"]);
}

#[test]
fn extract_labels_const() {
    let labels = extract_labels(r#"const "prefix" {}"#, "const");
    assert_eq!(labels, vec!["prefix"]);
}

// =============================================================================
// extract_blocks tests
// =============================================================================

#[test]
fn extract_empty_import() {
    let content = r#"import "oj/wok" {}
"#;
    let result = extract_blocks(content).unwrap();
    assert_eq!(result.imports.len(), 1);
    assert_eq!(result.imports[0].source, "oj/wok");
    assert!(result.imports[0].alias.is_none());
    assert!(result.imports[0].consts.is_empty());
    assert!(result.remaining.trim().is_empty());
}

#[test]
fn extract_import_with_alias() {
    let content = r#"import "oj/wok" "wok" {}
"#;
    let result = extract_blocks(content).unwrap();
    assert_eq!(result.imports.len(), 1);
    assert_eq!(result.imports[0].source, "oj/wok");
    assert_eq!(result.imports[0].alias.as_deref(), Some("wok"));
}

#[test]
fn extract_import_with_consts() {
    let content = r#"import "oj/wok" { const = { prefix = "oj" } }
"#;
    let result = extract_blocks(content).unwrap();
    assert_eq!(result.imports.len(), 1);
    assert_eq!(
        result.imports[0].consts.get("prefix"),
        Some(&"oj".to_string())
    );
}

#[test]
fn extract_import_with_alias_and_consts() {
    let content = r#"import "oj/wok" "wok" { const = { prefix = "oj", check = "make check" } }
"#;
    let result = extract_blocks(content).unwrap();
    assert_eq!(result.imports.len(), 1);
    assert_eq!(result.imports[0].source, "oj/wok");
    assert_eq!(result.imports[0].alias.as_deref(), Some("wok"));
    assert_eq!(
        result.imports[0].consts.get("prefix"),
        Some(&"oj".to_string())
    );
    assert_eq!(
        result.imports[0].consts.get("check"),
        Some(&"make check".to_string())
    );
}

#[test]
fn extract_const_required() {
    let content = r#"const "prefix" {}
"#;
    let result = extract_blocks(content).unwrap();
    assert_eq!(result.consts.len(), 1);
    assert_eq!(result.consts[0].name, "prefix");
    assert!(result.consts[0].default.is_none());
}

#[test]
fn extract_const_with_default() {
    let content = r#"const "check" { default = "true" }
"#;
    let result = extract_blocks(content).unwrap();
    assert_eq!(result.consts.len(), 1);
    assert_eq!(result.consts[0].name, "check");
    assert_eq!(result.consts[0].default.as_deref(), Some("true"));
}

#[test]
fn extract_preserves_remaining() {
    let content = r#"import "oj/wok" {}

command "test" {
  run = "echo test"
}
"#;
    let result = extract_blocks(content).unwrap();
    assert_eq!(result.imports.len(), 1);
    assert!(result.remaining.contains("command \"test\""));
    assert!(!result.remaining.contains("import"));
}

#[test]
fn extract_multiple_blocks() {
    let content = r#"const "prefix" {}
const "check" { default = "true" }

import "oj/wok" { const = { prefix = "oj" } }

command "test" {
  run = "echo test"
}
"#;
    let result = extract_blocks(content).unwrap();
    assert_eq!(result.consts.len(), 2);
    assert_eq!(result.imports.len(), 1);
    assert!(result.remaining.contains("command \"test\""));
    assert!(!result.remaining.contains("const "));
    assert!(!result.remaining.contains("import "));
}

// =============================================================================
// interpolate_consts tests
// =============================================================================

#[test]
fn interpolate_const_value() {
    let values: HashMap<String, String> = [("prefix".to_string(), "oj".to_string())]
        .into_iter()
        .collect();
    let result = interpolate_consts("wok ready -p ${const.prefix} -o json", &values);
    assert_eq!(result, "wok ready -p oj -o json");
}

#[test]
fn interpolate_const_shell_escapes() {
    let values: HashMap<String, String> = [("prefix".to_string(), "my$project".to_string())]
        .into_iter()
        .collect();
    let result = interpolate_consts("wok ready -p ${const.prefix} -o json", &values);
    assert_eq!(result, "wok ready -p my\\$project -o json");
}

#[test]
fn interpolate_raw_const() {
    let values: HashMap<String, String> = [("check".to_string(), "make check".to_string())]
        .into_iter()
        .collect();
    let result = interpolate_consts("run = \"${raw(const.check)}\"", &values);
    assert_eq!(result, "run = \"make check\"");
}

#[test]
fn interpolate_raw_const_not_escaped() {
    let values: HashMap<String, String> = [("check".to_string(), "echo $HOME".to_string())]
        .into_iter()
        .collect();
    let result = interpolate_consts("${raw(const.check)}", &values);
    assert_eq!(result, "echo $HOME");
}

#[test]
fn interpolate_unknown_const_left_alone() {
    let values: HashMap<String, String> = HashMap::new();
    let result = interpolate_consts("${const.unknown}", &values);
    assert_eq!(result, "${const.unknown}");
}

// =============================================================================
// validate_consts tests
// =============================================================================

#[test]
fn validate_consts_required_provided() {
    let defs = vec![ConstDef {
        name: "prefix".to_string(),
        default: None,
    }];
    let provided: HashMap<String, String> = [("prefix".to_string(), "oj".to_string())]
        .into_iter()
        .collect();
    let (values, warnings) = validate_consts(&defs, &provided, "oj/wok").unwrap();
    assert_eq!(values.get("prefix"), Some(&"oj".to_string()));
    assert!(warnings.is_empty());
}

#[test]
fn validate_consts_required_missing() {
    let defs = vec![ConstDef {
        name: "prefix".to_string(),
        default: None,
    }];
    let provided: HashMap<String, String> = HashMap::new();
    let err = validate_consts(&defs, &provided, "oj/wok").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("missing required const 'prefix'"),
        "got: {msg}"
    );
}

#[test]
fn validate_consts_default_used() {
    let defs = vec![ConstDef {
        name: "check".to_string(),
        default: Some("true".to_string()),
    }];
    let provided: HashMap<String, String> = HashMap::new();
    let (values, _) = validate_consts(&defs, &provided, "oj/wok").unwrap();
    assert_eq!(values.get("check"), Some(&"true".to_string()));
}

#[test]
fn validate_consts_default_overridden() {
    let defs = vec![ConstDef {
        name: "check".to_string(),
        default: Some("true".to_string()),
    }];
    let provided: HashMap<String, String> = [("check".to_string(), "make check".to_string())]
        .into_iter()
        .collect();
    let (values, _) = validate_consts(&defs, &provided, "oj/wok").unwrap();
    assert_eq!(values.get("check"), Some(&"make check".to_string()));
}

#[test]
fn validate_consts_unknown_warns() {
    let defs = vec![ConstDef {
        name: "prefix".to_string(),
        default: None,
    }];
    let provided: HashMap<String, String> = [
        ("prefix".to_string(), "oj".to_string()),
        ("extra".to_string(), "value".to_string()),
    ]
    .into_iter()
    .collect();
    let (_, warnings) = validate_consts(&defs, &provided, "oj/wok").unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(matches!(&warnings[0], ImportWarning::UnknownConst { name, .. } if name == "extra"));
}

// =============================================================================
// resolve_library tests
// =============================================================================

#[test]
fn resolve_known_libraries() {
    assert!(resolve_library("oj/wok").is_ok());
    assert!(resolve_library("oj/merge").is_ok());
}

#[test]
fn resolve_unknown_library() {
    let err = resolve_library("oj/unknown").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("unknown library"), "got: {msg}");
}

// =============================================================================
// merge_runbook tests
// =============================================================================

fn test_cmd(name: &str, run: &str) -> crate::CommandDef {
    crate::CommandDef {
        name: name.to_string(),
        description: None,
        args: crate::ArgSpec::default(),
        defaults: HashMap::new(),
        run: crate::RunDirective::Shell(run.to_string()),
    }
}

#[test]
fn merge_no_conflicts() {
    let mut target = Runbook::default();
    target
        .commands
        .insert("local-cmd".to_string(), test_cmd("local-cmd", "echo local"));

    let mut source = Runbook::default();
    source.commands.insert(
        "imported-cmd".to_string(),
        test_cmd("imported-cmd", "echo imported"),
    );

    let warnings = merge_runbook(&mut target, source, None, "test").unwrap();
    assert!(warnings.is_empty());
    assert!(target.commands.contains_key("local-cmd"));
    assert!(target.commands.contains_key("imported-cmd"));
}

#[test]
fn merge_local_overrides_import() {
    let mut target = Runbook::default();
    target
        .commands
        .insert("cmd".to_string(), test_cmd("cmd", "echo local"));

    let mut source = Runbook::default();
    source
        .commands
        .insert("cmd".to_string(), test_cmd("cmd", "echo imported"));

    let warnings = merge_runbook(&mut target, source, None, "test").unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        &warnings[0],
        ImportWarning::LocalOverride { entity_type: "command", name, .. } if name == "cmd"
    ));
    // Local wins
    assert_eq!(
        target.commands["cmd"].run.shell_command(),
        Some("echo local")
    );
}

#[test]
fn merge_with_alias_prefixes_names() {
    let mut target = Runbook::default();
    let mut source = Runbook::default();
    source
        .commands
        .insert("fix".to_string(), test_cmd("fix", "echo fix"));

    let warnings = merge_runbook(&mut target, source, Some("wok"), "test").unwrap();
    assert!(warnings.is_empty());
    assert!(target.commands.contains_key("wok.fix"));
    assert!(!target.commands.contains_key("fix"));
}

// =============================================================================
// parse_with_imports integration tests
// =============================================================================

#[test]
fn parse_import_oj_wok() {
    let content = r#"import "oj/wok" { const = { prefix = "oj" } }
"#;
    let (runbook, warnings) = parse_with_imports(content, Format::Hcl).unwrap();

    // Check unknown const warnings only
    for w in &warnings {
        assert!(
            !matches!(w, ImportWarning::UnknownConst { .. }),
            "unexpected unknown const warning: {}",
            w
        );
    }

    // Should have wok entities
    assert!(
        runbook.commands.contains_key("fix"),
        "missing 'fix' command"
    );
    assert!(
        runbook.commands.contains_key("chore"),
        "missing 'chore' command"
    );
    assert!(runbook.queues.contains_key("bugs"), "missing 'bugs' queue");
    assert!(
        runbook.queues.contains_key("chores"),
        "missing 'chores' queue"
    );
    assert!(runbook.workers.contains_key("bug"), "missing 'bug' worker");
    assert!(
        runbook.workers.contains_key("chore"),
        "missing 'chore' worker"
    );
    assert!(runbook.jobs.contains_key("bug"), "missing 'bug' job");
    assert!(runbook.jobs.contains_key("chore"), "missing 'chore' job");
    assert!(runbook.agents.contains_key("bugs"), "missing 'bugs' agent");
    assert!(
        runbook.agents.contains_key("chores"),
        "missing 'chores' agent"
    );
}

#[test]
fn parse_import_oj_wok_with_alias() {
    let content = r#"import "oj/wok" "wok" { const = { prefix = "oj" } }
"#;
    let (runbook, _) = parse_with_imports(content, Format::Hcl).unwrap();

    // All names should be prefixed with "wok."
    assert!(
        runbook.commands.contains_key("wok.fix"),
        "missing 'wok.fix' command"
    );
    assert!(
        runbook.queues.contains_key("wok.bugs"),
        "missing 'wok.bugs' queue"
    );
    assert!(
        runbook.workers.contains_key("wok.bug"),
        "missing 'wok.bug' worker"
    );
    assert!(
        runbook.jobs.contains_key("wok.bug"),
        "missing 'wok.bug' job"
    );
    assert!(
        runbook.agents.contains_key("wok.bugs"),
        "missing 'wok.bugs' agent"
    );
}

#[test]
fn parse_import_oj_merge() {
    let content = r#"import "oj/merge" {}
"#;
    let (runbook, _) = parse_with_imports(content, Format::Hcl).unwrap();

    assert!(
        runbook.commands.contains_key("merge"),
        "missing 'merge' command"
    );
    assert!(
        runbook.queues.contains_key("merges"),
        "missing 'merges' queue"
    );
    assert!(
        runbook.queues.contains_key("merge-conflicts"),
        "missing 'merge-conflicts' queue"
    );
    assert!(
        runbook.workers.contains_key("merge"),
        "missing 'merge' worker"
    );
    assert!(runbook.jobs.contains_key("merge"), "missing 'merge' job");
    assert!(
        runbook.agents.contains_key("conflicts"),
        "missing 'conflicts' agent"
    );
}

#[test]
fn parse_import_with_custom_check() {
    let content = r#"import "oj/wok" { const = { prefix = "oj", check = "make check" } }
"#;
    let (runbook, _) = parse_with_imports(content, Format::Hcl).unwrap();

    // The agent's on_dead gate should use "make check"
    let bugs_agent = runbook.agents.get("bugs").unwrap();
    assert_eq!(bugs_agent.on_dead.run(), Some("make check"));
}

#[test]
fn parse_import_missing_required_const() {
    let content = r#"import "oj/wok" {}
"#;
    let err = parse_with_imports(content, Format::Hcl).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("missing required const 'prefix'"),
        "got: {msg}"
    );
}

// =============================================================================
// available_libraries tests
// =============================================================================

#[test]
fn available_libraries_returns_all() {
    let libs = available_libraries();
    assert_eq!(libs.len(), 2);
    let sources: Vec<&str> = libs.iter().map(|l| l.source).collect();
    assert!(sources.contains(&"oj/wok"), "missing oj/wok");
    assert!(sources.contains(&"oj/merge"), "missing oj/merge");
}

#[test]
fn available_libraries_have_descriptions() {
    let libs = available_libraries();
    for lib in &libs {
        assert!(
            !lib.description.is_empty(),
            "library '{}' has empty description",
            lib.source
        );
    }
}

#[test]
fn available_libraries_parse_successfully() {
    let libs = available_libraries();
    for lib in &libs {
        let extracted = extract_blocks(lib.content).unwrap_or_else(|e| {
            panic!("failed to extract blocks from '{}': {}", lib.source, e);
        });
        let _runbook = crate::parser::parse_runbook_no_xref(&extracted.remaining, Format::Hcl)
            .unwrap_or_else(|e| {
                panic!("failed to parse library '{}': {}", lib.source, e);
            });
    }
}

#[test]
fn parse_no_imports_passthrough() {
    let content = r#"
command "test" {
  run = "echo test"
}
"#;
    let (runbook, warnings) = parse_with_imports(content, Format::Hcl).unwrap();
    assert!(warnings.is_empty());
    assert!(runbook.commands.contains_key("test"));
}
