# Plan: `oj pipeline show` Variable Truncation

## Overview

Add display formatting for pipeline variables in `oj pipeline show`. Variable values are capped at 80 characters with a `...` suffix and newlines replaced with literal `\n`. A `--verbose` flag shows full values with per-line indentation for readability. Only the CLI display layer changes — no protocol or daemon modifications.

## Project Structure

All changes are in the CLI crate:

```
crates/cli/src/commands/
├── pipeline.rs        # Main changes: Show enum variant, var display logic
└── pipeline_tests.rs  # New unit tests for formatting helpers
```

## Dependencies

None — uses only `std` string operations already available.

## Implementation Phases

### Phase 1: Add `format_var_value` helper function

Add a new helper function near the existing `truncate()` function in `pipeline.rs`:

```rust
fn format_var_value(value: &str, max_len: usize) -> String {
    // Replace newlines with literal \n
    let escaped = value.replace('\n', "\\n");
    if escaped.len() <= max_len {
        escaped
    } else {
        format!("{}...", &escaped[..max_len])
    }
}
```

This function:
1. Replaces `\n` with literal `\n` first (so the length check accounts for the escaped form)
2. Truncates to `max_len` characters and appends `...` if over the limit

**Verification:** Unit tests for the helper (Phase 3).

### Phase 2: Add `--verbose` flag to Show subcommand and update var display

Modify the `Show` variant in the `PipelineCommand` enum to accept a `--verbose` flag:

```rust
Show {
    /// Pipeline ID or name
    id: String,

    /// Show full variable values without truncation
    #[arg(long, short = 'v')]
    verbose: bool,
},
```

Update the `Show` match arm to destructure the new field and update the var display block (currently lines 340-345):

**Default (truncated) output:**
```rust
if !p.vars.is_empty() {
    println!("  Vars:");
    for (k, v) in &p.vars {
        println!("    {}: {}", k, format_var_value(v, 80));
    }
}
```

**Verbose output:**
```rust
if !p.vars.is_empty() {
    println!("  Vars:");
    for (k, v) in &p.vars {
        if v.contains('\n') {
            println!("    {}:", k);
            for line in v.lines() {
                println!("      {}", line);
            }
        } else {
            println!("    {}: {}", k, v);
        }
    }
}
```

In verbose mode:
- Single-line values print on the same line as the key: `    key: value`
- Multi-line values print the key alone, then each line indented two extra spaces under the key, making it easy to visually distinguish var boundaries

**Verification:** `cargo build --all` compiles. Manual `oj pipeline show <id>` vs `oj pipeline show -v <id>`.

### Phase 3: Add unit tests

Add tests to `pipeline_tests.rs`:

1. **`format_var_short_value_unchanged`** — value under 80 chars passes through (with newline escaping)
2. **`format_var_long_value_truncated`** — value over 80 chars gets `...` suffix
3. **`format_var_newlines_escaped`** — embedded newlines become literal `\n`
4. **`format_var_newlines_and_truncation`** — both transformations apply together; escaped form is what gets measured for length

Import `format_var_value` in the test module alongside existing imports.

**Verification:** `cargo test -p oj-cli`

### Phase 4: Run `make check`

Run the full verification suite:
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `quench check`
- `cargo test --all`
- `cargo build --all`
- `cargo audit`
- `cargo deny check licenses bans sources`

Fix any issues that arise.

## Key Implementation Details

- **Truncation boundary:** The 80-char limit applies to the *escaped* form of the value (after `\n` replacement). This means a value with many newlines will appear shorter than 80 raw characters because `\n` → `\\n` doubles the character count per newline.
- **UTF-8 safety:** The `&escaped[..max_len]` slice must land on a char boundary. Use `escaped.char_indices()` to find the correct byte offset for the 80th character, or alternatively use `escaped.chars().take(max_len).collect::<String>()`. The latter is simpler and correct.
- **JSON output unaffected:** The `OutputFormat::Json` branch (line 350-352) serializes the full `PipelineDetail` struct and is not changed — truncation is display-only.
- **Existing `truncate()` function:** Not reused because it returns `&str` without appending `...` or escaping newlines. The new `format_var_value` returns an owned `String` since it transforms the content.

## Verification Plan

1. **Unit tests** cover all formatting edge cases (short, long, newlines, combined)
2. **`make check`** passes (fmt, clippy, quench, test, build, audit, deny)
3. **Manual smoke test** (optional): run `oj pipeline show <id>` on a pipeline with long/multiline vars, confirm truncation; run with `-v` to confirm full output with indentation
