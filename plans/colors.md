# Plan: ANSI Color Support for oj CLI

## Overview

Add a `color` module to `crates/cli/src/` that defines the project's 4-color palette (matching wok/quench conventions) and a `should_colorize()` function. Wire the palette into clap's `Styles` API so all help output is automatically colored. The module is designed for reuse by future list/status formatting.

## Project Structure

```
crates/cli/src/
├── color.rs          # NEW — palette constants, should_colorize(), clap Styles builder
├── main.rs           # MODIFIED — apply color::styles() to clap Command
├── output.rs         # MODIFIED — re-export or delegate to color::should_colorize()
└── ...
```

## Dependencies

No new crate dependencies. Clap 4.5 already provides `clap::builder::styling::{Styles, Style, AnsiColor, Color, RgbColor}` — the 256-color index values are set via `Style::new().fg_color(Some(Color::Ansi256(N)))`. The existing `std::io::IsTerminal` covers TTY detection.

## Implementation Phases

### Phase 1: Create the `color` module

Create `crates/cli/src/color.rs` with:

1. **Palette constants** — a `codes` submodule with the 4 color values:

```rust
pub mod codes {
    /// Section headers: pastel cyan / steel blue (matches wok & quench)
    pub const HEADER: u8 = 74;
    /// Commands and literals: light grey
    pub const LITERAL: u8 = 250;
    /// Descriptions and context: medium grey
    pub const CONTEXT: u8 = 245;
    /// Muted / secondary text: darker grey
    pub const MUTED: u8 = 240;
}
```

2. **`should_colorize()`** — same logic as the existing `output::should_use_color()`, following the wok/quench pattern. Uses `OnceLock` for caching (quench pattern) since the result can't change during a process:

```rust
use std::io::IsTerminal;
use std::sync::OnceLock;

pub fn should_colorize() -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        if std::env::var("NO_COLOR").is_ok_and(|v| v == "1") {
            return false;
        }
        if std::env::var("COLOR").is_ok_and(|v| v == "1") {
            return true;
        }
        std::io::stdout().is_terminal()
    })
}
```

3. **Helper functions** for later use by list/status commands:

```rust
fn fg256(code: u8) -> String {
    format!("\x1b[38;5;{code}m")
}

const RESET: &str = "\x1b[0m";

pub fn header(text: &str) -> String {
    if should_colorize() { format!("{}{}{}", fg256(codes::HEADER), text, RESET) }
    else { text.to_string() }
}

pub fn literal(text: &str) -> String { /* same pattern with LITERAL */ }
pub fn context(text: &str) -> String { /* same pattern with CONTEXT */ }
pub fn muted(text: &str) -> String   { /* same pattern with MUTED */ }
```

4. **`styles()` builder** — returns a `clap::builder::styling::Styles` for use in the `#[command]` attribute:

```rust
use clap::builder::styling::{Style, Styles, Color};

pub fn styles() -> Styles {
    if !should_colorize() {
        return Styles::plain();
    }
    Styles::styled()
        .header(Style::new().fg_color(Some(Color::Ansi256(codes::HEADER))))
        .literal(Style::new().fg_color(Some(Color::Ansi256(codes::LITERAL))))
        .placeholder(Style::new().fg_color(Some(Color::Ansi256(codes::CONTEXT))))
}
```

Clap's `Styles` has five slots: `header`, `literal`, `placeholder`, `error`, and `valid`. We set three; `error` and `valid` keep their defaults (red and green).

### Phase 2: Wire into clap

In `crates/cli/src/main.rs`:

1. Add `mod color;` declaration.
2. Change the `#[command]` attribute on `Cli` to use the styles. Since `styles()` is not `const`, use clap's `Command::styles()` method at parse time instead of the derive attribute:

```rust
// Replace Cli::parse() with:
let cli = Cli::parse_from({
    // We need to use the augmented command
    use clap::CommandFactory;
    let cmd = Cli::command().styles(color::styles());
    cmd.get_matches()
});
// Actually: use Cli::from_arg_matches(&matches)
```

More precisely, change the call site in `run()`:

```rust
use clap::CommandFactory;
let matches = Cli::command().styles(color::styles()).get_matches();
let cli = Cli::from_arg_matches(&matches)?;
```

This applies the color styles to all help and error output without any custom help formatting.

### Phase 3: Migrate `output::should_use_color`

The existing `should_use_color()` in `output.rs` duplicates what `color::should_colorize()` does. Update it to delegate:

```rust
pub fn should_use_color() -> bool {
    crate::color::should_colorize()
}
```

This avoids breaking existing callers (`commands/pipeline.rs`, `commands/session.rs`) while unifying the source of truth.

### Phase 4: Add tests

Create `crates/cli/src/color_tests.rs` with unit tests:

1. **`styles_returns_styled_when_color_forced`** — set `COLOR=1`, verify `styles()` is not `Styles::plain()`.
2. **`styles_returns_plain_when_no_color`** — set `NO_COLOR=1`, verify plain styles.
3. **`helper_functions_produce_ansi_escapes`** — with `COLOR=1`, verify `header("foo")` contains `\x1b[38;5;74m`.
4. **`helper_functions_plain_when_disabled`** — with `NO_COLOR=1`, verify `header("foo") == "foo"`.

Note: Since `should_colorize()` uses `OnceLock`, tests that need different color states must either:
- Run in separate processes (use `#[test]` with `std::process::Command`), or
- Test the helper functions by testing `styles()` directly (which calls `should_colorize()` each time before the cache is set), or
- Add a `#[cfg(test)]` escape hatch that resets the cache.

The simplest approach: make the `OnceLock` cache `#[cfg(not(test))]` only, and have the test path re-evaluate each time. Alternatively, skip caching entirely (the check is cheap) and match wok's simpler non-cached pattern.

**Recommendation:** Skip the `OnceLock` cache for simplicity. The function is called a handful of times per process. Follow wok's non-cached pattern. This eliminates all test complexity around cached state.

## Key Implementation Details

- **Color values**: `HEADER=74` (steel blue), `LITERAL=250` (light grey), `CONTEXT=245` (medium grey), `MUTED=240` (dark grey). The first three match wok and quench exactly.
- **Detection priority**: `NO_COLOR=1` > `COLOR=1` > TTY check. Follows https://no-color.org/.
- **Clap integration**: Use `Command::styles()` method at runtime rather than derive attributes, since `styles()` needs to check environment variables.
- **No new dependencies**: Pure ANSI 256-color escape sequences for helpers; clap's built-in `Color::Ansi256` for help styles.
- **Module is reusable**: The `header()`, `literal()`, `context()`, `muted()` functions plus `codes::*` constants are available for future command output (lists, status dashboards, etc).

## Verification Plan

1. **`make check`** — must pass (fmt, clippy, tests, build, audit, deny).
2. **Manual verification**:
   - `oj --help` in a terminal → headers in steel blue, commands in light grey, placeholders in medium grey.
   - `oj pipeline --help` → same styling on subcommand help.
   - `NO_COLOR=1 oj --help` → plain uncolored output.
   - `COLOR=1 oj --help | cat` → colored output even though piped (forced).
   - Pipe without COLOR: `oj --help | cat` → plain output (not a TTY).
3. **Unit tests** in `color_tests.rs` cover detection logic and helper output.
