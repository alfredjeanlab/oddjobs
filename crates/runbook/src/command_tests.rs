// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

// Argument validation tests
#[test]
fn validate_required_positional_missing() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: parse_arg_spec("<name> <prompt>").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo".to_string()),
    };

    // Missing both required args
    let result = cmd.validate_args(&[], &HashMap::new());
    assert!(matches!(
        result,
        Err(ArgValidationError::MissingPositional(name)) if name == "name"
    ));

    // Missing second required arg
    let result = cmd.validate_args(&["foo".to_string()], &HashMap::new());
    assert!(matches!(
        result,
        Err(ArgValidationError::MissingPositional(name)) if name == "prompt"
    ));

    // All required args provided
    let result = cmd.validate_args(&["foo".to_string(), "bar".to_string()], &HashMap::new());
    assert!(result.is_ok());
}

#[test]
fn validate_required_positional_with_default() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: parse_arg_spec("<name>").unwrap(),
        defaults: [("name".to_string(), "default-name".to_string())]
            .into_iter()
            .collect(),
        run: RunDirective::Shell("echo".to_string()),
    };

    // Default satisfies requirement
    let result = cmd.validate_args(&[], &HashMap::new());
    assert!(result.is_ok());
}

#[test]
fn validate_required_option_missing() {
    let cmd = CommandDef {
        name: "deploy".to_string(),
        description: None,
        args: parse_arg_spec("--env <environment>").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("deploy.sh".to_string()),
    };

    // Missing required option
    let result = cmd.validate_args(&[], &HashMap::new());
    assert!(matches!(
        result,
        Err(ArgValidationError::MissingOption(name)) if name == "env"
    ));

    // Required option provided via named args
    let result = cmd.validate_args(
        &[],
        &[("env".to_string(), "prod".to_string())]
            .into_iter()
            .collect(),
    );
    assert!(result.is_ok());
}

#[test]
fn validate_required_variadic_missing() {
    let cmd = CommandDef {
        name: "copy".to_string(),
        description: None,
        args: parse_arg_spec("<files...>").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("cp".to_string()),
    };

    // Missing required variadic
    let result = cmd.validate_args(&[], &HashMap::new());
    assert!(matches!(
        result,
        Err(ArgValidationError::MissingVariadic(name)) if name == "files"
    ));

    // Required variadic provided
    let result = cmd.validate_args(&["file1".to_string()], &HashMap::new());
    assert!(result.is_ok());
}

#[test]
fn validate_optional_args_not_required() {
    let cmd = CommandDef {
        name: "test".to_string(),
        description: None,
        args: parse_arg_spec("[name] [-v/--verbose] [files...]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("test.sh".to_string()),
    };

    // All optional - should pass with no args
    let result = cmd.validate_args(&[], &HashMap::new());
    assert!(result.is_ok());
}

// ArgSpec parsing tests
#[test]
fn parse_simple_positional() {
    let spec = parse_arg_spec("<name> <prompt>").unwrap();
    assert_eq!(spec.positional.len(), 2);
    assert!(spec.positional[0].required);
    assert_eq!(spec.positional[0].name, "name");
    assert!(spec.positional[1].required);
    assert_eq!(spec.positional[1].name, "prompt");
}

#[test]
fn parse_optional_positional() {
    let spec = parse_arg_spec("<name> [description]").unwrap();
    assert!(spec.positional[0].required);
    assert!(!spec.positional[1].required);
}

#[test]
fn parse_flags_and_options() {
    let spec = parse_arg_spec("<env> [-t/--tag <version>] [-f/--force]").unwrap();
    assert_eq!(spec.positional.len(), 1);
    assert_eq!(spec.options.len(), 1);
    assert_eq!(spec.options[0].name, "tag");
    assert_eq!(spec.options[0].short, Some('t'));
    assert!(!spec.options[0].required);
    assert_eq!(spec.flags.len(), 1);
    assert_eq!(spec.flags[0].name, "force");
    assert_eq!(spec.flags[0].short, Some('f'));
}

#[test]
fn parse_variadic() {
    let spec = parse_arg_spec("<cmd> [args...]").unwrap();
    assert!(spec.variadic.is_some());
    assert!(!spec.variadic.as_ref().unwrap().required);
    assert_eq!(spec.variadic.as_ref().unwrap().name, "args");
}

#[test]
fn parse_required_variadic() {
    let spec = parse_arg_spec("<cmd> <files...>").unwrap();
    assert!(spec.variadic.is_some());
    assert!(spec.variadic.as_ref().unwrap().required);
    assert_eq!(spec.variadic.as_ref().unwrap().name, "files");
}

#[test]
fn parse_empty_spec() {
    let spec = parse_arg_spec("").unwrap();
    assert!(spec.positional.is_empty());
    assert!(spec.flags.is_empty());
    assert!(spec.options.is_empty());
    assert!(spec.variadic.is_none());
}

#[test]
fn parse_required_flag() {
    let spec = parse_arg_spec("--verbose").unwrap();
    assert_eq!(spec.flags.len(), 1);
    assert_eq!(spec.flags[0].name, "verbose");
}

#[test]
fn parse_required_option() {
    let spec = parse_arg_spec("--config <file>").unwrap();
    assert_eq!(spec.options.len(), 1);
    assert_eq!(spec.options[0].name, "config");
    assert!(spec.options[0].required);
}

#[test]
fn parse_complex_spec() {
    let spec = parse_arg_spec("<env> [-t/--tag <version>] [-f/--force] [targets...]").unwrap();
    assert_eq!(spec.positional.len(), 1);
    assert_eq!(spec.positional[0].name, "env");
    assert_eq!(spec.options.len(), 1);
    assert_eq!(spec.flags.len(), 1);
    assert!(spec.variadic.is_some());
}

#[test]
fn parse_error_variadic_not_last() {
    let result = parse_arg_spec("<files...> <other>");
    assert!(result.is_err());
}

#[test]
fn parse_error_optional_before_required() {
    let result = parse_arg_spec("[optional] <required>");
    assert!(result.is_err());
}

#[test]
fn parse_error_duplicate_name() {
    let result = parse_arg_spec("<name> <name>");
    assert!(result.is_err());
}

// RunDirective tests
#[test]
fn run_directive_shell() {
    let directive = RunDirective::Shell("echo hello".to_string());
    assert!(directive.is_shell());
    assert!(!directive.is_pipeline());
    assert_eq!(directive.shell_command(), Some("echo hello"));
}

#[test]
fn run_directive_pipeline() {
    let directive = RunDirective::Pipeline {
        pipeline: "build".to_string(),
    };
    assert!(directive.is_pipeline());
    assert!(!directive.is_shell());
    assert_eq!(directive.pipeline_name(), Some("build"));
}

#[test]
fn run_directive_agent() {
    let directive = RunDirective::Agent {
        agent: "planning".to_string(),
    };
    assert!(directive.is_agent());
    assert_eq!(directive.agent_name(), Some("planning"));
}

// TOML deserialization tests
#[test]
fn deserialize_shell_run() {
    #[derive(Deserialize)]
    struct Test {
        run: RunDirective,
    }
    let toml = r#"run = "echo hello""#;
    let test: Test = toml::from_str(toml).unwrap();
    assert!(test.run.is_shell());
    assert_eq!(test.run.shell_command(), Some("echo hello"));
}

#[test]
fn deserialize_pipeline_run() {
    #[derive(Deserialize)]
    struct Test {
        run: RunDirective,
    }
    let toml = r#"run = { pipeline = "build" }"#;
    let test: Test = toml::from_str(toml).unwrap();
    assert_eq!(test.run.pipeline_name(), Some("build"));
}

#[test]
fn deserialize_agent_run() {
    #[derive(Deserialize)]
    struct Test {
        run: RunDirective,
    }
    let toml = r#"run = { agent = "planning" }"#;
    let test: Test = toml::from_str(toml).unwrap();
    assert_eq!(test.run.agent_name(), Some("planning"));
}

#[test]
fn deserialize_arg_spec_string() {
    #[derive(Deserialize)]
    struct Test {
        args: ArgSpec,
    }
    let toml = r#"args = "<name> <prompt>""#;
    let test: Test = toml::from_str(toml).unwrap();
    assert_eq!(test.args.positional.len(), 2);
    assert_eq!(test.args.positional[0].name, "name");
}

// CommandDef tests
#[test]
fn command_parse_args() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: ArgSpec {
            positional: vec![
                ArgDef {
                    name: "name".to_string(),
                    required: true,
                },
                ArgDef {
                    name: "prompt".to_string(),
                    required: true,
                },
            ],
            flags: Vec::new(),
            options: Vec::new(),
            variadic: None,
        },
        defaults: [("branch".to_string(), "main".to_string())]
            .into_iter()
            .collect(),
        run: RunDirective::Pipeline {
            pipeline: "build".to_string(),
        },
    };

    let result = cmd.parse_args(
        &["feature".to_string(), "Add login".to_string()],
        &HashMap::new(),
    );

    assert_eq!(result.get("name"), Some(&"feature".to_string()));
    assert_eq!(result.get("prompt"), Some(&"Add login".to_string()));
    assert_eq!(result.get("branch"), Some(&"main".to_string()));
}

#[test]
fn command_named_overrides() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: ArgSpec {
            positional: vec![ArgDef {
                name: "name".to_string(),
                required: true,
            }],
            flags: Vec::new(),
            options: vec![OptionDef {
                name: "branch".to_string(),
                short: None,
                required: false,
            }],
            variadic: None,
        },
        defaults: [("branch".to_string(), "main".to_string())]
            .into_iter()
            .collect(),
        run: RunDirective::Pipeline {
            pipeline: "build".to_string(),
        },
    };

    let result = cmd.parse_args(
        &["feature".to_string()],
        &[("branch".to_string(), "develop".to_string())]
            .into_iter()
            .collect(),
    );

    assert_eq!(result.get("branch"), Some(&"develop".to_string()));
}

#[test]
fn command_variadic_args() {
    let cmd = CommandDef {
        name: "deploy".to_string(),
        description: None,
        args: ArgSpec {
            positional: vec![ArgDef {
                name: "env".to_string(),
                required: true,
            }],
            flags: Vec::new(),
            options: Vec::new(),
            variadic: Some(VariadicDef {
                name: "targets".to_string(),
                required: false,
            }),
        },
        defaults: HashMap::new(),
        run: RunDirective::Shell("deploy.sh".to_string()),
    };

    let result = cmd.parse_args(
        &["prod".to_string(), "api".to_string(), "worker".to_string()],
        &HashMap::new(),
    );

    assert_eq!(result.get("env"), Some(&"prod".to_string()));
    assert_eq!(result.get("targets"), Some(&"api worker".to_string()));
}

#[test]
fn validate_unexpected_positional() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: parse_arg_spec("<name>").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo".to_string()),
    };

    // Too many positional args
    let result = cmd.validate_args(&["foo".to_string(), "extra".to_string()], &HashMap::new());
    assert!(matches!(
        result,
        Err(ArgValidationError::UnexpectedPositional(arg)) if arg == "extra"
    ));
}

#[test]
fn validate_unexpected_positional_variadic_ok() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: parse_arg_spec("<name> [extras...]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo".to_string()),
    };

    // Extra args allowed with variadic
    let result = cmd.validate_args(
        &[
            "foo".to_string(),
            "extra1".to_string(),
            "extra2".to_string(),
        ],
        &HashMap::new(),
    );
    assert!(result.is_ok());
}

#[test]
fn validate_unknown_option() {
    let cmd = CommandDef {
        name: "deploy".to_string(),
        description: None,
        args: parse_arg_spec("<env> [--tag <v>]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("deploy.sh".to_string()),
    };

    // Unknown named arg
    let result = cmd.validate_args(
        &["prod".to_string()],
        &[("unknown".to_string(), "value".to_string())]
            .into_iter()
            .collect(),
    );
    assert!(matches!(
        result,
        Err(ArgValidationError::UnknownOption(name)) if name == "unknown"
    ));
}

#[test]
fn validate_known_option_by_name() {
    let cmd = CommandDef {
        name: "deploy".to_string(),
        description: None,
        args: parse_arg_spec("<env> [--tag <v>]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("deploy.sh".to_string()),
    };

    // Known option is OK
    let result = cmd.validate_args(
        &["prod".to_string()],
        &[("tag".to_string(), "v1.0".to_string())]
            .into_iter()
            .collect(),
    );
    assert!(result.is_ok());
}

#[test]
fn validate_positional_by_name() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: parse_arg_spec("<name> <prompt>").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo".to_string()),
    };

    // Providing positional arg by name is allowed
    let result = cmd.validate_args(
        &["feature".to_string()],
        &[("prompt".to_string(), "Add login".to_string())]
            .into_iter()
            .collect(),
    );
    assert!(result.is_ok());
}

#[test]
fn validate_flag_by_name() {
    let cmd = CommandDef {
        name: "deploy".to_string(),
        description: None,
        args: parse_arg_spec("<env> [-f/--force]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("deploy.sh".to_string()),
    };

    // Providing flag by name is allowed
    let result = cmd.validate_args(
        &["prod".to_string()],
        &[("force".to_string(), "true".to_string())]
            .into_iter()
            .collect(),
    );
    assert!(result.is_ok());
}

// usage_line tests
#[test]
fn usage_line_empty_spec() {
    let spec = parse_arg_spec("").unwrap();
    assert_eq!(spec.usage_line(), "");
}

#[test]
fn usage_line_positional_only() {
    let spec = parse_arg_spec("<name> <prompt>").unwrap();
    assert_eq!(spec.usage_line(), "<name> <prompt>");
}

#[test]
fn usage_line_optional_positional() {
    let spec = parse_arg_spec("<name> [description]").unwrap();
    assert_eq!(spec.usage_line(), "<name> [description]");
}

#[test]
fn usage_line_with_variadic() {
    let spec = parse_arg_spec("<cmd> [args...]").unwrap();
    assert_eq!(spec.usage_line(), "<cmd> [args...]");
}

#[test]
fn usage_line_required_variadic() {
    let spec = parse_arg_spec("<files...>").unwrap();
    assert_eq!(spec.usage_line(), "<files...>");
}

#[test]
fn usage_line_with_options_and_flags() {
    let spec = parse_arg_spec("<name> [--base <branch>] [--rebase] [--new <folder>]").unwrap();
    assert_eq!(
        spec.usage_line(),
        "<name> [--base <base>] [--new <new>] [--rebase]"
    );
}

#[test]
fn usage_line_mixed() {
    let spec = parse_arg_spec("<env> [-t/--tag <version>] [-f/--force] [targets...]").unwrap();
    assert_eq!(
        spec.usage_line(),
        "<env> [targets...] [--tag <tag>] [--force]"
    );
}

// description field tests
#[test]
fn command_def_description_none_by_default() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: None,
        args: ArgSpec::default(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo".to_string()),
    };
    assert!(cmd.description.is_none());
}

#[test]
fn command_def_description_some() {
    let cmd = CommandDef {
        name: "build".to_string(),
        description: Some("Run a build pipeline".to_string()),
        args: ArgSpec::default(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo".to_string()),
    };
    assert_eq!(cmd.description.as_deref(), Some("Run a build pipeline"));
}

#[test]
fn deserialize_description_from_hcl() {
    let hcl = r#"
command "build" {
  description = "Run a build pipeline"
  args = "<name>"
  run  = "echo build"
}
"#;
    let runbook: crate::Runbook = hcl::from_str(hcl).unwrap();
    let cmd = runbook.commands.get("build").unwrap();
    assert_eq!(cmd.description.as_deref(), Some("Run a build pipeline"));
}

// split_raw_args tests
#[test]
fn split_raw_args_flags_after_positional() {
    let spec = parse_arg_spec("<name> [--new <value>] [--base <branch>]").unwrap();
    let raw: Vec<String> = vec!["kanban", "--new", "kanban-board", "--base", "develop"]
        .into_iter()
        .map(String::from)
        .collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["kanban"]);
    assert_eq!(named.get("new"), Some(&"kanban-board".to_string()));
    assert_eq!(named.get("base"), Some(&"develop".to_string()));
}

#[test]
fn split_raw_args_flags_before_positional() {
    let spec = parse_arg_spec("<name> [--new <value>]").unwrap();
    let raw: Vec<String> = vec!["--new", "kanban-board", "kanban"]
        .into_iter()
        .map(String::from)
        .collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["kanban"]);
    assert_eq!(named.get("new"), Some(&"kanban-board".to_string()));
}

#[test]
fn split_raw_args_intermixed() {
    let spec = parse_arg_spec("<name> <prompt> [-f/--force] [--base <branch>]").unwrap();
    let raw: Vec<String> = vec!["kanban", "--force", "--base", "main", "build the thing"]
        .into_iter()
        .map(String::from)
        .collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["kanban", "build the thing"]);
    assert_eq!(named.get("force"), Some(&"true".to_string()));
    assert_eq!(named.get("base"), Some(&"main".to_string()));
}

#[test]
fn split_raw_args_short_flags() {
    let spec = parse_arg_spec("<env> [-f/--force] [-t/--tag <version>]").unwrap();
    let raw: Vec<String> = vec!["prod", "-f", "-t", "v1.0"]
        .into_iter()
        .map(String::from)
        .collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["prod"]);
    assert_eq!(named.get("force"), Some(&"true".to_string()));
    assert_eq!(named.get("tag"), Some(&"v1.0".to_string()));
}

#[test]
fn split_raw_args_double_dash_stops_parsing() {
    let spec = parse_arg_spec("<name> [--force]").unwrap();
    let raw: Vec<String> = vec!["kanban", "--", "--force"]
        .into_iter()
        .map(String::from)
        .collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["kanban", "--force"]);
    assert!(named.is_empty());
}

#[test]
fn split_raw_args_unknown_flags_kept_as_positional() {
    let spec = parse_arg_spec("<name> [extras...]").unwrap();
    let raw: Vec<String> = vec!["kanban", "--unknown", "value"]
        .into_iter()
        .map(String::from)
        .collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["kanban", "--unknown", "value"]);
    assert!(named.is_empty());
}

#[test]
fn split_raw_args_no_flags() {
    let spec = parse_arg_spec("<name> <prompt>").unwrap();
    let raw: Vec<String> = vec!["kanban", "build the thing"]
        .into_iter()
        .map(String::from)
        .collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["kanban", "build the thing"]);
    assert!(named.is_empty());
}
