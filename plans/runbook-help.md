# Runbook Help

Auto-generate `--help` output for runbook commands so that `oj run <command> --help` produces a natural CLI help page.

## Overview

When a user runs `oj run build --help`, the system should parse the runbook file's leading comment block and the command's `args` definition to produce clap-style help output. The description comes from the file's leading comment (up to the first blank line), the Usage/Options sections are generated from the `ArgSpec`, and remaining comment content appears in a Description section with `Usage:` lines rewritten as `Examples:`.

**Example output:**
```
Feature development workflow: init → plan-agent → implement-agent → rebase → done

Usage: oj run build <name> <instructions> [--base <branch>] [--rebase] [--new <folder>]

Arguments:
  <name>
  <instructions>

Options:
      --base <branch>    [default: main]
      --rebase
      --new <folder>

Description:
  Examples:
    oj run build <name> <instructions>
```

## Project Structure

Key files to modify:

```
crates/
├── runbook/src/
│   ├── find.rs          # Add comment extraction alongside command collection
│   ├── command.rs        # Add help formatting methods to CommandDef/ArgSpec
│   └── lib.rs            # Export new types
└── cli/src/
    └── commands/run.rs   # Intercept --help, call help formatter
```

## Dependencies

No new external dependencies. Uses existing `hcl`, `serde`, and standard library.

## Implementation Phases

### Phase 1: Extract leading comments from runbook files

Add a function in `crates/runbook/src/find.rs` that reads the raw file content and extracts the leading `#`-comment block before the first HCL block. This is a simple text operation — no HCL parser changes needed.

**New function in `find.rs`:**

```rust
/// Extract the leading comment block from a runbook file's raw content.
///
/// Reads lines starting with `#`, strips the `# ` prefix, and returns:
/// - `short`: text up to the first blank comment line (two consecutive newlines)
/// - `long`: remaining comment text after the blank line
///
/// Returns `None` if the file has no leading comment block.
pub fn extract_file_comment(content: &str) -> Option<FileComment> {
    let mut lines = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            // Strip "# " or "#" prefix
            let text = trimmed.strip_prefix("# ").unwrap_or(
                trimmed.strip_prefix('#').unwrap_or("")
            );
            lines.push(text.to_string());
        } else if trimmed.is_empty() && lines.is_empty() {
            // Skip leading blank lines
            continue;
        } else {
            // Non-comment line — stop
            break;
        }
    }

    if lines.is_empty() {
        return None;
    }

    // Split on first blank line (empty string in our vec)
    let split_pos = lines.iter().position(|l| l.is_empty());
    let (short_lines, long_lines) = match split_pos {
        Some(pos) => (&lines[..pos], &lines[pos + 1..]),
        None => (lines.as_slice(), &[][..]),
    };

    Some(FileComment {
        short: short_lines.join("\n"),
        long: long_lines.join("\n"),
    })
}

pub struct FileComment {
    pub short: String,
    pub long: String,
}
```

The `short` field becomes the command description (first paragraph). The `long` field is additional content shown in the Description section.

Update `collect_all_commands` to also return the `FileComment` for each runbook file, or add a parallel function `find_runbook_file_by_command` that returns `(PathBuf, Format)` so the CLI can read the raw file itself. The simpler approach: add a new function that returns the comment alongside the command:

```rust
/// Find a command definition and its runbook file comment.
pub fn find_command_with_comment(
    runbook_dir: &Path,
    command_name: &str,
) -> Result<Option<(CommandDef, Option<FileComment>)>, FindError>
```

This scans runbook files, finds the one defining the command, reads its raw content, and calls `extract_file_comment`.

**Verification:** Unit test that parses `build.hcl`'s comment block and gets:
- short: `"Build Runbook\nFeature development workflow: init → plan-agent → implement-agent → rebase → done"`
- long: `"Usage:\n  oj run build <name> <instructions>"`

### Phase 2: Format help output from CommandDef

Add a `format_help` method (or free function) in `crates/runbook/src/command.rs` that generates clap-style help text given a `CommandDef`, command name, and optional `FileComment`.

**New method on `CommandDef`:**

```rust
impl CommandDef {
    /// Format a --help page for this command.
    pub fn format_help(&self, command_name: &str, comment: Option<&FileComment>) -> String {
        let mut out = String::new();

        // 1. Short description (from comment or description field)
        if let Some(desc) = self.description.as_deref()
            .or(comment.map(|c| c.short.as_str()))
        {
            out.push_str(desc);
            out.push_str("\n\n");
        }

        // 2. Usage line
        let usage = self.args.usage_line();
        if usage.is_empty() {
            writeln!(out, "Usage: oj run {command_name}").unwrap();
        } else {
            writeln!(out, "Usage: oj run {command_name} {usage}").unwrap();
        }

        // 3. Arguments section (positional + variadic)
        if !self.args.positional.is_empty() || self.args.variadic.is_some() {
            out.push_str("\nArguments:\n");
            for arg in &self.args.positional {
                let label = if arg.required {
                    format!("  <{}>", arg.name)
                } else {
                    format!("  [{}]", arg.name)
                };
                if let Some(default) = self.defaults.get(&arg.name) {
                    if !default.is_empty() {
                        writeln!(out, "{label:<24} [default: {default}]").unwrap();
                        continue;
                    }
                }
                writeln!(out, "{label}").unwrap();
            }
            if let Some(v) = &self.args.variadic {
                let label = if v.required {
                    format!("  <{}...>", v.name)
                } else {
                    format!("  [{}...]", v.name)
                };
                writeln!(out, "{label}").unwrap();
            }
        }

        // 4. Options section (flags + options)
        if !self.args.options.is_empty() || !self.args.flags.is_empty() {
            out.push_str("\nOptions:\n");
            for opt in &self.args.options {
                let short = opt.short.map(|c| format!("-{c}, ")).unwrap_or_default();
                let label = format!("  {short}--{} <{}>", opt.name, opt.name);
                if let Some(default) = self.defaults.get(&opt.name) {
                    if !default.is_empty() {
                        writeln!(out, "{label:<24} [default: {default}]").unwrap();
                    } else {
                        writeln!(out, "{label}").unwrap();
                    }
                } else {
                    writeln!(out, "{label}").unwrap();
                }
            }
            for flag in &self.args.flags {
                let short = flag.short.map(|c| format!("-{c}, ")).unwrap_or_default();
                let label = format!("  {short}--{}", flag.name);
                writeln!(out, "{label}").unwrap();
            }
        }

        // 5. Description section (long comment, with Usage: → Examples:)
        if let Some(comment) = comment {
            if !comment.long.is_empty() {
                let rewritten = comment.long.lines().map(|line| {
                    if line.trim_start().starts_with("Usage:") {
                        line.replacen("Usage:", "Examples:", 1)
                    } else {
                        line.to_string()
                    }
                }).collect::<Vec<_>>().join("\n");
                out.push_str("\nDescription:\n");
                for line in rewritten.lines() {
                    writeln!(out, "  {line}").unwrap();
                }
            }
        }

        out
    }
}
```

Export `FileComment` from `crates/runbook/src/lib.rs`.

**Verification:** Unit test that builds a `CommandDef` matching `build.hcl` and asserts the formatted output matches the expected help page.

### Phase 3: Intercept `--help` in the CLI

Modify `crates/cli/src/commands/run.rs` to detect `--help` in the args and print the generated help instead of running the command.

**Detection logic in `handle()`:** After resolving the command name but before validation, check if `--help` or `-h` is in `args.args`:

```rust
pub async fn handle(args: RunArgs, ...) -> Result<()> {
    let Some(ref command) = args.command else {
        return print_available_commands(project_root);
    };

    // Check for --help before anything else
    if args.args.iter().any(|a| a == "--help" || a == "-h") {
        return print_command_help(project_root, command, args.runbook.as_deref());
    }

    // ... existing logic
}
```

**New function `print_command_help`:**

```rust
fn print_command_help(project_root: &Path, command: &str, runbook_file: Option<&str>) -> Result<()> {
    let runbook_dir = project_root.join(".oj/runbooks");

    // Load command definition
    let runbook = crate::load_runbook(project_root, command, runbook_file)?;
    let cmd_def = runbook.get_command(command)
        .ok_or_else(|| anyhow::anyhow!("unknown command: {}", command))?;

    // Load file comment
    let comment = oj_runbook::find_command_with_comment(&runbook_dir, command)
        .ok()
        .flatten()
        .and_then(|(_, comment)| comment);

    eprint!("{}", cmd_def.format_help(command, comment.as_ref()));
    std::process::exit(0);
}
```

Also update `print_available_commands` to populate descriptions from file comments when `cmd.description` is `None`, so the command listing shows short descriptions too.

**Verification:** Manual test: `oj run build --help` produces the expected output. `oj run fix --help` produces help for the fix command.

### Phase 4: Populate descriptions in command listing

Update `collect_all_commands` in `find.rs` to populate `CommandDef.description` from the file's leading comment when the field isn't set in the HCL. This way `print_available_commands` (the `oj run` listing) shows descriptions without changes to the CLI code.

The first line of `short` (up to `\n`) becomes the description. For `build.hcl` this would be `"Build Runbook"`. Alternatively, skip the title line and use the second line: `"Feature development workflow: init → plan-agent → implement-agent → rebase → done"`.

Approach: Use the first non-title line. If the short comment is a single line, use it as-is. If multi-line, skip the first line (treated as a title) and use the second.

```rust
// In collect_all_commands, after parsing:
let comment = extract_file_comment(&content);
for (name, mut cmd) in runbook.commands {
    if cmd.description.is_none() {
        if let Some(ref comment) = comment {
            // Use second line of short description, or first if only one line
            let desc_line = comment.short.lines()
                .nth(1)
                .or_else(|| comment.short.lines().next())
                .unwrap_or("");
            if !desc_line.is_empty() {
                cmd.description = Some(desc_line.to_string());
            }
        }
    }
    commands.push((name, cmd));
}
```

**Verification:** `oj run` (no args) shows descriptions for each command.

## Key Implementation Details

### Comment parsing strategy

The leading comment is extracted as raw text before HCL parsing. This avoids any dependency on HCL comment AST support (which `hcl-rs` doesn't expose through serde). The split on "two consecutive newlines" maps to a blank `#` line in the file:

```hcl
# Build Runbook                          ← short (line 1: title)
# Feature development workflow: ...      ← short (line 2: description)
#                                        ← blank line = separator
# Usage:                                 ← long (additional content)
#   oj run build <name> <instructions>   ← long
```

### Usage: → Examples: rewrite

The instructions specify replacing `^Usage:` lines with `Examples:`. This applies only in the Description (long comment) section, since we generate our own Usage section. The regex-free approach: check if a trimmed line starts with `"Usage:"` and replace the first occurrence.

### Defaults display

Defaults from `CommandDef.defaults` are shown inline as `[default: value]` after each option/argument, matching clap's style. Empty-string defaults (like `rebase = ""` which is a boolean flag's false state) are not displayed.

### Precedence

`CommandDef.description` field (explicit in HCL) takes precedence over the extracted file comment. This lets runbooks opt into a custom short description that differs from the file comment.

### No TOML comment extraction

TOML files don't have a standard way to preserve comments through `toml-rs`. For TOML runbooks, users should set the `description` field explicitly. HCL comment extraction is the primary path since HCL is the recommended format.

## Verification Plan

1. **Unit tests in `find_tests.rs`:**
   - `extract_file_comment` with multi-paragraph comments
   - `extract_file_comment` with single-line comment
   - `extract_file_comment` with no comment
   - `extract_file_comment` with leading blank lines
   - `find_command_with_comment` returns correct data

2. **Unit tests in `command_tests.rs`:**
   - `format_help` with full args (positional, options, flags, defaults)
   - `format_help` with no args
   - `format_help` with description field set (overrides comment)
   - `format_help` with long comment containing `Usage:` lines
   - `format_help` with variadic args

3. **Integration test:**
   - Write a test runbook with a known comment block
   - Run `oj run testcmd --help` and assert output matches expected format

4. **Manual verification:**
   - `oj run build --help` — full help with args, options, defaults, description
   - `oj run fix --help` — simple help with one positional arg
   - `oj run` — command listing shows descriptions from file comments
   - `oj run nonexistent --help` — error: unknown command

5. **`make check`** must pass (fmt, clippy, tests, build, audit, deny).
