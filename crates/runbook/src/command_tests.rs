// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use super::*;

// Argument validation tests
#[test]
fn validate_required_positional_missing() {
    let cmd = CommandDef {
        name: "build".to_string(),

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

        args: parse_arg_spec("<name>").unwrap(),
        defaults: [("name".to_string(), "default-name".to_string())].into_iter().collect(),
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
    let result =
        cmd.validate_args(&[], &[("env".to_string(), "prod".to_string())].into_iter().collect());
    assert!(result.is_ok());
}

#[test]
fn validate_required_variadic_missing() {
    let cmd = CommandDef {
        name: "copy".to_string(),

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

#[yare::parameterized(
    variadic_not_last            = { "<files...> <other>" },
    optional_before_required     = { "[optional] <required>" },
    duplicate_name               = { "<name> <name>" },
    ellipsis_outside_not_last    = { "<files>... <other>" },
    ellipsis_on_flag             = { "[--flag]..." },
    ellipsis_on_option           = { "[--opt <val>]..." },
)]
fn parse_arg_spec_errors(spec: &str) {
    assert!(parse_arg_spec(spec).is_err());
}

// RunDirective tests
#[test]
fn run_directive_shell() {
    let directive = RunDirective::Shell("echo hello".to_string());
    assert!(directive.is_shell());
    assert!(!directive.is_job());
    assert_eq!(directive.shell_command(), Some("echo hello"));
}

#[test]
fn run_directive_job() {
    let directive = RunDirective::Job { job: "build".to_string() };
    assert!(directive.is_job());
    assert!(!directive.is_shell());
    assert_eq!(directive.job_name(), Some("build"));
}

#[test]
fn run_directive_agent() {
    let directive = RunDirective::Agent { agent: "planning".to_string(), attach: None };
    assert!(directive.is_agent());
    assert_eq!(directive.agent_name(), Some("planning"));
    assert_eq!(directive.attach(), None);
}

// TOML deserialization tests

#[derive(Deserialize)]
struct RunTest {
    run: RunDirective,
}

#[test]
fn deserialize_shell_run() {
    let t: RunTest = toml::from_str(r#"run = "echo hello""#).unwrap();
    assert!(t.run.is_shell());
    assert_eq!(t.run.shell_command(), Some("echo hello"));
}

#[test]
fn deserialize_job_run() {
    let t: RunTest = toml::from_str(r#"run = { job = "build" }"#).unwrap();
    assert_eq!(t.run.job_name(), Some("build"));
}

#[test]
fn deserialize_crew() {
    let t: RunTest = toml::from_str(r#"run = { agent = "planning" }"#).unwrap();
    assert_eq!(t.run.agent_name(), Some("planning"));
    assert_eq!(t.run.attach(), None);
}

#[test]
fn deserialize_crew_with_attach_true() {
    let t: RunTest = toml::from_str(r#"run = { agent = "planning", attach = true }"#).unwrap();
    assert_eq!(t.run, RunDirective::Agent { agent: "planning".to_string(), attach: Some(true) });
    assert_eq!(t.run.attach(), Some(true));
}

#[test]
fn deserialize_crew_with_attach_false() {
    let t: RunTest = toml::from_str(r#"run = { agent = "planning", attach = false }"#).unwrap();
    assert_eq!(t.run, RunDirective::Agent { agent: "planning".to_string(), attach: Some(false) });
    assert_eq!(t.run.attach(), Some(false));
}

#[test]
fn deserialize_crew_hcl_with_attach() {
    let hcl = r#"
command "mayor" {
  run = { agent = "mayor", attach = true }
}
"#;
    let runbook: crate::Runbook = hcl::from_str(hcl).unwrap();
    let cmd = runbook.commands.get("mayor").unwrap();
    assert_eq!(cmd.run.agent_name(), Some("mayor"));
    assert_eq!(cmd.run.attach(), Some(true));
}

#[test]
fn attach_accessor_returns_none_for_non_agent() {
    assert_eq!(RunDirective::Shell("echo".to_string()).attach(), None);
    assert_eq!(RunDirective::Job { job: "build".to_string() }.attach(), None);
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

        args: ArgSpec {
            positional: vec![
                ArgDef { name: "name".to_string(), required: true },
                ArgDef { name: "prompt".to_string(), required: true },
            ],
            flags: Vec::new(),
            options: Vec::new(),
            variadic: None,
        },
        defaults: [("branch".to_string(), "main".to_string())].into_iter().collect(),
        run: RunDirective::Job { job: "build".to_string() },
    };

    let result = cmd.parse_args(&["feature".to_string(), "Add login".to_string()], &HashMap::new());

    assert_eq!(result.get("name"), Some(&"feature".to_string()));
    assert_eq!(result.get("prompt"), Some(&"Add login".to_string()));
    assert_eq!(result.get("branch"), Some(&"main".to_string()));
}

#[test]
fn command_named_overrides() {
    let cmd = CommandDef {
        name: "build".to_string(),

        args: ArgSpec {
            positional: vec![ArgDef { name: "name".to_string(), required: true }],
            flags: Vec::new(),
            options: vec![OptionDef { name: "branch".to_string(), short: None, required: false }],
            variadic: None,
        },
        defaults: [("branch".to_string(), "main".to_string())].into_iter().collect(),
        run: RunDirective::Job { job: "build".to_string() },
    };

    let result = cmd.parse_args(
        &["feature".to_string()],
        &[("branch".to_string(), "develop".to_string())].into_iter().collect(),
    );

    assert_eq!(result.get("branch"), Some(&"develop".to_string()));
}

#[test]
fn command_variadic_args() {
    let cmd = CommandDef {
        name: "deploy".to_string(),

        args: ArgSpec {
            positional: vec![ArgDef { name: "env".to_string(), required: true }],
            flags: Vec::new(),
            options: Vec::new(),
            variadic: Some(VariadicDef { name: "targets".to_string(), required: false }),
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

        args: parse_arg_spec("<name> [extras...]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo".to_string()),
    };

    // Extra args allowed with variadic
    let result = cmd.validate_args(
        &["foo".to_string(), "extra1".to_string(), "extra2".to_string()],
        &HashMap::new(),
    );
    assert!(result.is_ok());
}

#[test]
fn validate_unknown_option() {
    let cmd = CommandDef {
        name: "deploy".to_string(),

        args: parse_arg_spec("<env> [--tag <v>]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("deploy.sh".to_string()),
    };

    // Unknown named arg
    let result = cmd.validate_args(
        &["prod".to_string()],
        &[("unknown".to_string(), "value".to_string())].into_iter().collect(),
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

        args: parse_arg_spec("<env> [--tag <v>]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("deploy.sh".to_string()),
    };

    // Known option is OK
    let result = cmd.validate_args(
        &["prod".to_string()],
        &[("tag".to_string(), "v1.0".to_string())].into_iter().collect(),
    );
    assert!(result.is_ok());
}

#[test]
fn validate_positional_by_name() {
    let cmd = CommandDef {
        name: "build".to_string(),

        args: parse_arg_spec("<name> <prompt>").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("echo".to_string()),
    };

    // Providing positional arg by name is allowed
    let result = cmd.validate_args(
        &["feature".to_string()],
        &[("prompt".to_string(), "Add login".to_string())].into_iter().collect(),
    );
    assert!(result.is_ok());
}

#[test]
fn validate_flag_by_name() {
    let cmd = CommandDef {
        name: "deploy".to_string(),

        args: parse_arg_spec("<env> [-f/--force]").unwrap(),
        defaults: HashMap::new(),
        run: RunDirective::Shell("deploy.sh".to_string()),
    };

    // Providing flag by name is allowed
    let result = cmd.validate_args(
        &["prod".to_string()],
        &[("force".to_string(), "true".to_string())].into_iter().collect(),
    );
    assert!(result.is_ok());
}

// usage_line tests
#[yare::parameterized(
    empty_spec         = { "",                                                          "" },
    positional_only    = { "<name> <prompt>",                                           "<name> <prompt>" },
    optional_pos       = { "<name> [description]",                                      "<name> [description]" },
    with_variadic      = { "<cmd> [args...]",                                           "<cmd> [args...]" },
    required_variadic  = { "<files...>",                                                "<files...>" },
    options_and_flags  = { "<name> [--base <branch>] [--rebase] [--new <folder>]",      "<name> [--base <base>] [--new <new>] [--rebase]" },
    mixed              = { "<env> [-t/--tag <version>] [-f/--force] [targets...]",      "<env> [targets...] [--tag <tag>] [--force]" },
)]
fn usage_line(spec_str: &str, expected: &str) {
    assert_eq!(parse_arg_spec(spec_str).unwrap().usage_line(), expected);
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
    let raw: Vec<String> =
        vec!["--new", "kanban-board", "kanban"].into_iter().map(String::from).collect();

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
    let raw: Vec<String> = vec!["prod", "-f", "-t", "v1.0"].into_iter().map(String::from).collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["prod"]);
    assert_eq!(named.get("force"), Some(&"true".to_string()));
    assert_eq!(named.get("tag"), Some(&"v1.0".to_string()));
}

#[test]
fn split_raw_args_double_dash_stops_parsing() {
    let spec = parse_arg_spec("<name> [--force]").unwrap();
    let raw: Vec<String> = vec!["kanban", "--", "--force"].into_iter().map(String::from).collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["kanban", "--force"]);
    assert!(named.is_empty());
}

#[test]
fn split_raw_args_unknown_flags_kept_as_positional() {
    let spec = parse_arg_spec("<name> [extras...]").unwrap();
    let raw: Vec<String> =
        vec!["kanban", "--unknown", "value"].into_iter().map(String::from).collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["kanban", "--unknown", "value"]);
    assert!(named.is_empty());
}

#[test]
fn split_raw_args_no_flags() {
    let spec = parse_arg_spec("<name> <prompt>").unwrap();
    let raw: Vec<String> =
        vec!["kanban", "build the thing"].into_iter().map(String::from).collect();

    let (positional, named) = spec.split_raw_args(&raw);
    assert_eq!(positional, vec!["kanban", "build the thing"]);
    assert!(named.is_empty());
}

// Tests for alternative variadic syntax with ellipsis outside brackets
#[test]
fn parse_variadic_ellipsis_outside_required() {
    let spec = parse_arg_spec("<cmd> <files>...").unwrap();
    assert!(spec.variadic.is_some());
    assert!(spec.variadic.as_ref().unwrap().required);
    assert_eq!(spec.variadic.as_ref().unwrap().name, "files");
}

#[test]
fn parse_variadic_ellipsis_outside_optional() {
    let spec = parse_arg_spec("<cmd> [args]...").unwrap();
    assert!(spec.variadic.is_some());
    assert!(!spec.variadic.as_ref().unwrap().required);
    assert_eq!(spec.variadic.as_ref().unwrap().name, "args");
}

#[test]
fn parse_variadic_both_syntaxes_equivalent() {
    let spec1 = parse_arg_spec("<files...>").unwrap();
    let spec2 = parse_arg_spec("<files>...").unwrap();
    assert_eq!(spec1.variadic.as_ref().unwrap().name, spec2.variadic.as_ref().unwrap().name);
    assert_eq!(
        spec1.variadic.as_ref().unwrap().required,
        spec2.variadic.as_ref().unwrap().required
    );
}

#[test]
fn parse_variadic_optional_both_syntaxes_equivalent() {
    let spec1 = parse_arg_spec("[files...]").unwrap();
    let spec2 = parse_arg_spec("[files]...").unwrap();
    assert_eq!(spec1.variadic.as_ref().unwrap().name, spec2.variadic.as_ref().unwrap().name);
    assert_eq!(
        spec1.variadic.as_ref().unwrap().required,
        spec2.variadic.as_ref().unwrap().required
    );
}

#[test]
fn usage_line_variadic_ellipsis_outside() {
    // Both syntaxes should produce the same canonical usage line (ellipsis inside brackets)
    let spec = parse_arg_spec("<cmd> <files>...").unwrap();
    assert_eq!(spec.usage_line(), "<cmd> <files...>");
}
