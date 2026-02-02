# Plan: Friendly `oj run` with No Arguments

## Overview

When `oj run` is invoked without a `<COMMAND>` argument, show a friendly listing of available commands discovered from runbooks instead of clap's default error. The output preserves the existing `Usage:` and `For more information` lines but inserts an `Available Commands:` section showing each command name, its args, and a brief description derived from the runbook's `run` directive.

## Project Structure

Key files to modify:

```
crates/runbook/src/find.rs     # Add collect_all_commands() function
crates/runbook/src/lib.rs      # Export new function
crates/cli/src/commands/run.rs  # Make `command` optional, handle missing case
crates/cli/src/main.rs          # Adjust dispatch for optional command
```

## Dependencies

No new external dependencies required. Uses existing `oj_runbook` crate for runbook parsing and discovery.

## Implementation Phases

### Phase 1: Add `collect_all_commands` to `oj_runbook`

Add a new public function in `crates/runbook/src/find.rs` that scans all runbook files and collects every `CommandDef`:

```rust
/// Scan `.oj/runbooks/` and collect all command definitions.
/// Returns a sorted vec of (command_name, CommandDef) pairs.
/// Skips runbooks that fail to parse (logs warnings).
pub fn collect_all_commands(runbook_dir: &Path) -> Result<Vec<(String, CommandDef)>, FindError> {
    if !runbook_dir.exists() {
        return Ok(Vec::new());
    }
    let files = collect_runbook_files(runbook_dir)?;
    let mut commands = Vec::new();
    for (path, format) in files {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping unreadable runbook");
                continue;
            }
        };
        let runbook = match parse_runbook_with_format(&content, format) {
            Ok(rb) => rb,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping invalid runbook");
                continue;
            }
        };
        for (name, cmd) in runbook.commands {
            commands.push((name, cmd));
        }
    }
    commands.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(commands)
}
```

Export from `crates/runbook/src/lib.rs`:
```rust
pub use find::{collect_all_commands, find_runbook_by_command, ...};
```

**Verification:** Unit test that creates a temp dir with two runbook files, calls `collect_all_commands`, and verifies all commands are returned sorted.

### Phase 2: Add `usage_line` method to `ArgSpec`

Add a method to `ArgSpec` in `crates/runbook/src/command.rs` that formats the args as a usage string (for display in the listing):

```rust
impl ArgSpec {
    /// Format as a usage string, e.g. "<name> <instructions> [--base <branch>]"
    pub fn usage_line(&self) -> String {
        let mut parts = Vec::new();
        for arg in &self.positional {
            if arg.required {
                parts.push(format!("<{}>", arg.name));
            } else {
                parts.push(format!("[{}]", arg.name));
            }
        }
        if let Some(v) = &self.variadic {
            if v.required {
                parts.push(format!("<{}...>", v.name));
            } else {
                parts.push(format!("[{}...]", v.name));
            }
        }
        for opt in &self.options {
            if opt.required {
                parts.push(format!("--{} <{}>", opt.name, opt.name));
            } else {
                parts.push(format!("[--{} <{}>]", opt.name, opt.name));
            }
        }
        for flag in &self.flags {
            parts.push(format!("[--{}]", flag.name));
        }
        parts.join(" ")
    }
}
```

**Verification:** Unit tests for `usage_line()` with various arg specs.

### Phase 3: Add `description` field to `CommandDef`

Add an optional `description` field to `CommandDef` so runbooks can provide a short summary:

```rust
pub struct CommandDef {
    #[serde(default)]
    pub name: String,
    /// Short description for help text (e.g., "Run a build pipeline")
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub args: ArgSpec,
    // ... existing fields
}
```

This is optional and backward-compatible. When absent, the listing shows just the command name and args without a description suffix.

**Verification:** Parse a runbook with `description = "..."` and verify it roundtrips.

### Phase 4: Make `command` optional in `RunArgs` and handle the no-args case

In `crates/cli/src/commands/run.rs`:

1. Change `command: String` to `command: Option<String>` in `RunArgs`.
2. Add a function to print the friendly listing:

```rust
fn print_available_commands(project_root: &Path) -> Result<()> {
    let runbook_dir = project_root.join(".oj/runbooks");
    let commands = oj_runbook::collect_all_commands(&runbook_dir)
        .unwrap_or_default();

    eprintln!("Usage: oj run <COMMAND> [ARGS]...");
    eprintln!();

    if !commands.is_empty() {
        eprintln!("Available Commands:");
        for (name, cmd) in &commands {
            let args_str = cmd.args.usage_line();
            let line = if args_str.is_empty() {
                format!("  {name}")
            } else {
                format!("  {name} {args_str}")
            };
            if let Some(desc) = &cmd.description {
                // Right-pad to align descriptions
                eprintln!("  {line:<40} {desc}");
            } else {
                eprintln!("{line}");
            }
        }
        eprintln!();
    }

    eprintln!("For more information, try '--help'.");
    std::process::exit(2);
}
```

3. In `handle()`, check if `command` is `None` and call `print_available_commands` early.

In `crates/cli/src/main.rs`, the dispatch for `Commands::Run(args)` stays the same — the `handle` function itself will detect the missing command.

**Verification:** Run `cargo run -p oj -- run` (or the built binary) with no args and verify the output format.

### Phase 5: Add tests

1. **Unit test in `find_tests.rs`**: Test `collect_all_commands` with multiple runbook files containing different commands, verify sorted output and that unparseable files are skipped.

2. **Unit test in `command_tests.rs`**: Test `ArgSpec::usage_line()` for empty spec, positional-only, flags, options, variadic, and mixed specs.

3. **Integration consideration**: The CLI behavior can be verified manually or via a shell-level test that runs `oj run` with no args and checks exit code 2 and output contains "Available Commands:".

## Key Implementation Details

- **`command` field becomes `Option<String>`**: Use clap's `#[arg(default_missing_value)]` or simply make it `Option<String>`. The key is that clap should not error on missing command — the handler does its own checking.
- **`collect_all_commands` reuses `collect_runbook_files`**: This private function already handles recursive directory scanning. The new function just iterates all files and extracts commands instead of searching for a specific one.
- **Output goes to stderr**: Follows clap's convention of writing help/error to stderr, and exits with code 2 (same as clap's missing-arg exit code).
- **`description` field is optional**: Existing runbooks without it still work. The listing just shows the command name and args. This can be populated later for better help text.
- **Deduplication**: If the same command name appears in multiple runbooks, `collect_all_commands` should keep only the first occurrence (or warn). The existing `find_runbook_by_command` already errors on duplicates, so duplicates would be a pre-existing configuration error. For the listing, just show all found entries — the user will see the duplicate and can fix it.

## Verification Plan

1. `cargo build --all` — compiles cleanly
2. `cargo test --all` — all existing tests pass
3. `cargo clippy --all-targets --all-features -- -D warnings` — no warnings
4. Manual test: `oj run` with no args shows:
   ```
   Usage: oj run <COMMAND> [ARGS]...

   Available Commands:
     build <name> <instructions> [--base <branch>] [--rebase] [--new <folder>]
     chore <description>
     fix <description>

   For more information, try '--help'.
   ```
5. Manual test: `oj run build` still works as before (proceeds to validate args)
6. `make check` passes
