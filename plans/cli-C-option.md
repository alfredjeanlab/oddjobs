# Plan: Add `-C` Option and Standardize `--project` as Top-Level CLI Options

## Overview

Add a new `-C <dir>` top-level option that changes the working directory before any project root discovery or command execution — analogous to `git -C`. Simultaneously, promote the existing `--project` flag from per-subcommand duplication to a single top-level global option. Refactor the namespace resolution logic so both `-C` and `--project` are handled uniformly in `main()` rather than scattered across every command handler.

**Semantic distinction:**
- `-C <dir>` — "Run as if `oj` was invoked from `<dir>`." Changes cwd, which affects project root discovery, namespace resolution, runbook loading, and `invoke_dir`.
- `--project <name>` — "Select project `<name>` by namespace." Overrides only the namespace; does not change cwd or project root discovery. (Today this exists as per-command `#[arg(long = "project")]`; behavior stays the same, just lifted to top-level.)

## Project Structure

Key files to modify:

```
crates/cli/src/
├── main.rs              # Add -C and --project to Cli struct; centralize resolution
├── commands/
│   ├── queue.rs         # Remove per-command --project fields
│   ├── pipeline.rs      # Remove per-command --project fields
│   ├── agent.rs         # Remove per-command --project fields
│   ├── worker.rs        # Remove per-command --project fields
│   ├── cron.rs          # Remove per-command --project fields
│   ├── decision.rs      # Remove per-command --project fields
│   └── run.rs           # (no --project today, but receives updated invoke_dir)
├── main_tests.rs        # Add tests for -C and --project parsing
```

## Dependencies

No new external dependencies. Uses existing `clap` features (`global = true`).

## Implementation Phases

### Phase 1: Add `-C` and `--project` as top-level global options

Add both fields to the `Cli` struct in `main.rs` with `global = true`, following the pattern of the existing `-o/--output` option.

**File: `crates/cli/src/main.rs`**

```rust
#[derive(Parser)]
#[command(
    name = "oj",
    version,
    disable_version_flag = true,
    about = "Odd Jobs - Agentic development automation"
)]
struct Cli {
    /// Change to <dir> before doing anything
    #[arg(short = 'C', global = true, value_name = "DIR")]
    directory: Option<PathBuf>,

    /// Project namespace override
    #[arg(long = "project", global = true)]
    project: Option<String>,

    /// Output format
    #[arg(
        short = 'o',
        long = "output",
        value_enum,
        default_value_t,
        global = true
    )]
    output: OutputFormat,

    #[command(subcommand)]
    command: Option<Commands>,
}
```

**Milestone:** `oj -C /some/dir status` and `oj --project foo queue list` parse correctly. `oj --help` shows both new flags.

### Phase 2: Apply `-C` early in `run()` and refactor project context resolution

In `main.rs::run()`, apply `-C` by changing the process working directory before any project root discovery. Then resolve namespace centrally, applying `--project` override logic once.

**File: `crates/cli/src/main.rs`**

```rust
async fn run() -> Result<()> {
    let matches = cli_command().get_matches();
    let cli = Cli::from_arg_matches(&matches)?;
    let format = cli.output;

    // Apply -C: change working directory early, before project root discovery
    if let Some(ref dir) = cli.directory {
        let canonical = std::fs::canonicalize(dir)
            .map_err(|e| anyhow::anyhow!("cannot change to directory '{}': {}", dir.display(), e))?;
        std::env::set_current_dir(&canonical)
            .map_err(|e| anyhow::anyhow!("cannot change to directory '{}': {}", canonical.display(), e))?;
    }

    let command = match cli.command { /* ... existing ... */ };

    // Handle daemon/env commands (no project context needed)
    // ...

    // Discover project context (now from potentially-changed cwd)
    let project_root = find_project_root();
    let invoke_dir = std::env::current_dir().unwrap_or_else(|_| project_root.clone());

    // Centralized namespace resolution:
    //   --project flag > OJ_NAMESPACE env > auto-resolved from project root
    let namespace = resolve_effective_namespace(cli.project.as_deref(), &project_root);

    // Dispatch commands...
}

/// Resolve the effective namespace using the standard priority chain:
///   --project flag > OJ_NAMESPACE env > project root resolution
fn resolve_effective_namespace(project: Option<&str>, project_root: &Path) -> String {
    if let Some(p) = project {
        return p.to_string();
    }
    if let Ok(ns) = std::env::var("OJ_NAMESPACE") {
        if !ns.is_empty() {
            return ns;
        }
    }
    oj_core::namespace::resolve_namespace(project_root)
}
```

Also update `cli_command()` since it calls `find_project_root()` for help text — apply `-C` there too via manual arg scanning (before clap parses), or accept that help text may not reflect `-C` (it only affects `oj run` available commands listing, which is a cosmetic detail).

**Milestone:** `oj -C /path/to/project run build` discovers the project root from `/path/to/project` and runs correctly. `oj --project myproject queue list` resolves namespace to `myproject`.

### Phase 3: Remove per-command `--project` fields from subcommands

Remove the `#[arg(long = "project")]` field from every subcommand variant that currently has it, and remove the per-handler namespace resolution boilerplate. Commands will receive the already-resolved namespace from `main()`.

**Affected commands and their per-variant `project` fields:**

| File | Variants with `project: Option<String>` |
|---|---|
| `queue.rs` | `Push`, `List`, `Items`, `Drop`, `Logs`, `Retry`, `Drain` (7 variants) |
| `pipeline.rs` | `List`, `Prune` (2 variants) |
| `agent.rs` | `List` (1 variant) |
| `worker.rs` | `Start`, `Stop`, `Restart`, `Logs`, `List`, `Prune` (6 variants) |
| `cron.rs` | `Start`, `Stop`, `Restart`, `Once`, `Logs`, `List`, `Prune` (7 variants) |
| `decision.rs` | `List` (1 variant) |

**For each command handler**, remove the inline namespace resolution pattern:
```rust
// DELETE this pattern from every match arm:
let effective_namespace = project
    .or_else(|| std::env::var("OJ_NAMESPACE").ok().filter(|s| !s.is_empty()))
    .unwrap_or_else(|| namespace.to_string());
```

And replace with direct use of the `namespace` parameter (already resolved in `main()`).

**Handler signature changes** — some handlers that currently receive separate `project_root` and `namespace` may have their signatures simplified. For example, `pipeline::handle()` currently doesn't receive `namespace`/`project_root` at all (it reads `--project` from its own args); it will now need to receive `namespace`:

```rust
// Before:
pub async fn handle(command: PipelineCommand, client: &DaemonClient, format: OutputFormat)

// After:
pub async fn handle(command: PipelineCommand, client: &DaemonClient, namespace: &str, format: OutputFormat)
```

Similarly, `agent::handle()` and `decision::handle()` will use the namespace passed from `main()` instead of resolving it internally.

**Milestone:** `grep -r 'long = "project"' crates/cli/src/commands/` returns no results. All namespace resolution happens in `main.rs::resolve_effective_namespace()`.

### Phase 4: Propagate `--project` to `find_project_root()` for help text (cosmetic)

The `cli_command()` function calls `find_project_root()` to generate `oj run` help text showing available commands. With `-C`, users may want `oj -C /other/project run --help` to show that project's commands.

Since `cli_command()` is called before `Cli::from_arg_matches()`, we need to manually scan for `-C` in `std::env::args()`:

```rust
fn cli_command() -> clap::Command {
    // Check for -C in raw args to discover correct project root for help text
    let project_root = find_project_root_from_args();
    let run_help = commands::run::available_commands_help(&project_root);
    // ...
}

/// Find project root, honoring a -C flag if present in raw argv.
fn find_project_root_from_args() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "-C" {
            if let Some(dir) = args.get(i + 1) {
                if let Ok(canonical) = std::fs::canonicalize(dir) {
                    // Temporarily discover project root from this directory
                    return find_project_root_from(canonical);
                }
            }
        }
    }
    find_project_root()
}
```

Refactor `find_project_root()` to accept an optional starting directory:

```rust
fn find_project_root() -> PathBuf {
    let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    find_project_root_from(start)
}

fn find_project_root_from(start: PathBuf) -> PathBuf {
    let mut current = start;
    loop {
        if current.join(".oj").is_dir() {
            return resolve_main_worktree(&current).unwrap_or(current);
        }
        if !current.pop() {
            return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        }
    }
}
```

**Milestone:** `oj -C /other/project run --help` shows commands from `/other/project/.oj/runbooks/`.

### Phase 5: Add tests

Add unit tests to `main_tests.rs` covering the new top-level options:

1. **`-C` parsing:** Verify `oj -C /tmp status` parses without error.
2. **`-C` with invalid dir:** Verify `oj -C /nonexistent status` produces a clear error.
3. **`--project` parsing:** Verify `oj --project myproj queue list` parses correctly.
4. **`-C` and `--project` together:** Verify both can be specified simultaneously.
5. **Help text:** Verify `oj --help` shows both `-C` and `--project` in the output.
6. **Backward compatibility:** Verify that subcommand-level `--project` no longer parses (it's been removed), or if kept for backward compat, that it conflicts/is silently ignored.

```rust
#[test]
fn directory_flag_short_c() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "-C", "/tmp", "status"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert_eq!(cli.directory.unwrap(), PathBuf::from("/tmp"));
}

#[test]
fn project_flag_global() {
    let matches = cli_command()
        .try_get_matches_from(["oj", "--project", "myproj", "queue", "list"])
        .unwrap();
    let cli = Cli::from_arg_matches(&matches).unwrap();
    assert_eq!(cli.project.unwrap(), "myproj");
}

#[test]
fn help_shows_directory_and_project_flags() {
    let mut buf = Vec::new();
    cli_command().write_help(&mut buf).unwrap();
    let help = String::from_utf8(buf).unwrap();
    assert!(help.contains("-C"), "help should show -C flag");
    assert!(help.contains("--project"), "help should show --project flag");
}
```

Add a `resolve_effective_namespace` unit test:

```rust
#[test]
fn namespace_resolution_priority() {
    // --project flag wins over everything
    let ns = resolve_effective_namespace(Some("override"), Path::new("/dummy"));
    assert_eq!(ns, "override");
}
```

**Milestone:** `cargo test --all` passes. All new flags are covered.

## Key Implementation Details

### Why `set_current_dir` for `-C`

Using `std::env::set_current_dir()` early in `run()` is the simplest approach because:
- `find_project_root()` uses `std::env::current_dir()` internally
- Shell command execution (`execute_shell_inline`) uses `project_root` as cwd
- `invoke_dir` is captured from `current_dir()` — with `-C` applied first, it naturally reflects the user's intent
- All downstream code "just works" without threading a separate `start_dir` parameter through every function

This matches how `git -C` works: it changes directory before doing anything else.

### `--project` as namespace-only override

The existing `--project` behavior across commands is purely a namespace override — it never changes the working directory or project root discovery. Promoting it to a top-level global option preserves this semantic exactly while eliminating ~24 duplicate `#[arg(long = "project")]` declarations and ~24 copies of the namespace resolution boilerplate.

### Interaction between `-C` and `--project`

Both can be used together. They are orthogonal:
- `-C /path/to/project` changes cwd → changes project root discovery → changes auto-resolved namespace
- `--project foo` overrides the namespace to `foo` regardless of what project root was discovered

Example: `oj -C /path/to/project --project custom-name queue push myq --var x=1`
- Project root discovered from `/path/to/project`
- Namespace is `custom-name` (overridden by `--project`)
- Runbooks loaded from `/path/to/project/.oj/runbooks/`

### Error handling for `-C`

If the directory doesn't exist or isn't accessible, fail immediately with a clear error message before any other processing. Use `std::fs::canonicalize()` to resolve symlinks and validate existence in one step.

## Verification Plan

1. **Unit tests** (Phase 5): Parsing tests in `main_tests.rs` for flag combinations and help output.
2. **`make check`**: Ensure `cargo fmt`, `cargo clippy`, and `cargo test --all` pass.
3. **Manual smoke tests:**
   - `oj --help` shows `-C` and `--project`
   - `oj -C /some/project run --help` shows that project's commands
   - `oj -C /some/project run build` discovers the correct project and runs
   - `oj --project myproj queue list` filters by namespace
   - `oj -C /nonexistent status` gives a clear error
   - Existing `OJ_NAMESPACE` env var behavior is preserved
   - `oj queue list` (no flags) still works as before
4. **Backward compatibility**: Verify that no existing scripts break — the per-command `--project` flag is removed, so any scripts using e.g. `oj queue list --project foo` must use `oj --project foo queue list` instead. This is a breaking change but is the explicit goal of standardization.
