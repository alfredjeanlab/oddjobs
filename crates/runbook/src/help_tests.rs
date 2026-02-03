// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use std::collections::HashMap;

use crate::command::{parse_arg_spec, ArgSpec, CommandDef, RunDirective};
use crate::find::FileComment;

#[test]
fn format_help_full_args() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: parse_arg_spec("<name> <instructions> [--base <branch>] [--rebase] [--new <folder>]")
            .unwrap(),
        defaults: [
            ("base".to_string(), "main".to_string()),
            ("rebase".to_string(), String::new()),
            ("new".to_string(), String::new()),
        ]
        .into_iter()
        .collect(),
        run: RunDirective::Pipeline {
            pipeline: "build".to_string(),
        },
    };

    let comment = FileComment {
        short:
            "Build Runbook\nFeature development workflow: init → plan → implement → rebase → done"
                .to_string(),
        long: "Usage:\n  oj run build <name> <instructions>".to_string(),
    };

    let help = cmd.format_help("build", Some(&comment));
    assert!(help.contains("Build Runbook"));
    assert!(help.contains("Feature development workflow"));
    assert!(help.contains("Usage: oj run build"));
    assert!(help.contains("Arguments:"));
    assert!(help.contains("  <name>"));
    assert!(help.contains("  <instructions>"));
    assert!(help.contains("Options:"));
    assert!(help.contains("--base <base>"));
    assert!(help.contains("[default: main]"));
    assert!(help.contains("--rebase"));
    assert!(help.contains("--new <new>"));
    assert!(help.contains("Description:"));
    assert!(help.contains("Examples:"));
}

#[test]
fn format_help_no_args() {
    let cmd = CommandDef {
        name: "test".to_string(),
        description: None,
        args: ArgSpec::default(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo test".to_string()),
    };

    let help = cmd.format_help("test", None);
    assert!(help.contains("Usage: oj run test\n"));
    assert!(!help.contains("Arguments:"));
    assert!(!help.contains("Options:"));
    assert!(!help.contains("Description:"));
}

#[test]
fn format_help_description_overrides_comment() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: Some("Explicit description".to_string()),
        args: parse_arg_spec("<name>").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo".to_string()),
    };

    let comment = FileComment {
        short: "Comment description".to_string(),
        long: String::new(),
    };

    let help = cmd.format_help("build", Some(&comment));
    assert!(help.starts_with("Explicit description\n"));
    assert!(!help.contains("Comment description"));
}

#[test]
fn format_help_usage_rewritten_to_examples() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: ArgSpec::default(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo".to_string()),
    };

    let comment = FileComment {
        short: "Title".to_string(),
        long: "Usage:\n  oj run build foo\nUsage:\n  oj run build bar".to_string(),
    };

    let help = cmd.format_help("build", Some(&comment));
    assert!(help.contains("Examples:"));
}

#[test]
fn format_help_with_variadic() {
    let cmd = CommandDef {
        name: "deploy".to_string(),
        description: None,
        args: parse_arg_spec("<env> [targets...]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("deploy.sh".to_string()),
    };

    let help = cmd.format_help("deploy", None);
    assert!(help.contains("  <env>"));
    assert!(help.contains("  [targets...]"));
}

#[test]
fn format_help_with_short_options() {
    let cmd = CommandDef {
        name: "deploy".to_string(),
        description: None,
        args: parse_arg_spec("<env> [-t/--tag <version>] [-f/--force]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("deploy.sh".to_string()),
    };

    let help = cmd.format_help("deploy", None);
    assert!(help.contains("-t, --tag <tag>"));
    assert!(help.contains("-f, --force"));
}

#[test]
fn format_help_empty_default_not_shown() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: parse_arg_spec("[--rebase]").unwrap(),
        defaults: [("rebase".to_string(), String::new())]
            .into_iter()
            .collect(),
        run: RunDirective::Shell("echo".to_string()),
    };

    let help = cmd.format_help("build", None);
    assert!(help.contains("--rebase"));
    assert!(!help.contains("[default:"));
}
