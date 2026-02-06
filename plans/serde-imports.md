# Plan: Make `import`/`const` blocks normal serde-deserializable HCL

## Problem

The import system uses text-level block extraction (`extract_blocks()`) to find
and remove `import`/`const` blocks before hcl-rs sees the content. This is
unnecessary — hcl-rs 0.18's serde deserializer treats `${...}` as literal
strings, so these blocks can be regular fields on the `Runbook` struct.

## Syntax change

The second label on import blocks moves into the body:

```hcl
# Before
import "oj/wok" "wok" { const = { prefix = "oj" } }

# After
import "oj/wok" { alias = "wok", const = { prefix = "oj" } }
```

No change needed for `const` blocks — `const "prefix" {}` is already
single-label and fits `HashMap<String, ConstDef>`.

No live `.oj/runbooks/*.hcl` files use import syntax yet. Only tests and
built-in libraries need updating.

## Steps

### 1. Add `imports` and `consts` to `Runbook` struct (`parser.rs`)

```rust
#[serde(default, alias = "import")]
pub imports: HashMap<String, ImportDef>,
#[serde(default, alias = "const")]
pub consts: HashMap<String, ConstDef>,
```

These are `#[serde(default)]` so existing files without imports/consts parse
fine. The `deny_unknown_fields` attribute stays — these are now known fields.

### 2. Refactor `ImportDef` and `ConstDef` to be serde-derivable (`import.rs`)

**ImportDef** — `source` removed (it's the HashMap key), `alias` moves from
label to field:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportDef {
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default, rename = "const")]
    pub consts: HashMap<String, String>,
}
```

Add a `source()` helper or accept the key from callers where needed.

**ConstDef** — `name` removed (it's the HashMap key):

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConstDef {
    #[serde(default)]
    pub default: Option<String>,
}
```

### 3. Delete text-level extraction code (`import.rs`)

Remove these functions and types:
- `extract_blocks()`
- `find_block_start()`
- `find_closing_brace()`
- `extract_labels()`
- `ExtractResult` struct
- `ImportBody` and `ConstBody` serde helpers

### 4. Rewrite `parse_with_imports()` pipeline (`import.rs`)

New flow:

```
1. parse_runbook_no_xref(content, format) → Runbook
   (imports & consts are now populated fields)
2. Take runbook.imports, clear the field
3. For each import (source, import_def):
   a. resolve_library(source) → library text
   b. parse_runbook_no_xref(library_text, Hcl) → library Runbook
      (gets const definitions from library.consts; ${const.name} in
       string values are just literal strings)
   c. validate_consts(&library.consts, &import_def.consts, source)
   d. interpolate_consts(library_text, &resolved_values) → interpolated text
   e. parse_runbook_no_xref(interpolated, Hcl) → final library Runbook
   f. Clear consts/imports from final library Runbook
   g. merge_runbook(target, library, import_def.alias, source)
4. validate_cross_refs(&merged)
```

The const value interpolation still operates on text (step 3d) because
`${const.name}` patterns need to be resolved before string values are finalized.
This is template substitution, not block extraction, so it's the right level.

### 5. Update `validate_consts()` signature (`import.rs`)

Change from `defs: &[ConstDef]` to `defs: &HashMap<String, ConstDef>`. The
const name comes from the HashMap key instead of from a `.name` field.

### 6. Update `collect_runbook_summaries()` in `find.rs`

Currently calls `extract_blocks()` to get imports separately from parsing.
Change to: parse with `parse_runbook_no_xref()`, read `runbook.imports`, then
clear imports/consts from the runbook before collecting entity names.

The `RunbookSummary.imports` field type changes from `Vec<ImportDef>` to
`HashMap<String, ImportDef>` (or a wrapper).

### 7. Update CLI `runbook.rs` consumers

- `imported_command_names()`: iterate `HashMap<String, ImportDef>` — key is
  source, `.alias` is a field on the value.
- `extract_const_defs()`: parse library as Runbook, read `.consts` field
  instead of calling `extract_blocks()`.
- `handle_show()`: same — parse library, read `.consts`.
- `format_const_summary()` / `format_consts_json()`: adapt to
  `HashMap<String, ConstDef>` (name from key).
- JSON output in `handle_list()`: `i.source` → the HashMap key.

### 8. Update public API in `lib.rs`

- Remove re-export of `extract_blocks` and `ExtractResult`
- Keep re-exports of `ImportDef`, `ConstDef`, `ImportWarning`, etc.
- Add re-export of new Runbook methods if any

### 9. Update tests

**import_tests.rs:**
- Delete `extract_labels` tests (function removed)
- Delete `extract_blocks` tests (function removed)
- Update `validate_consts` tests for new `HashMap<String, ConstDef>` signature
- Update `parse_with_imports` tests: change `import "oj/wok" "wok" { ... }`
  to `import "oj/wok" { alias = "wok", ... }`
- Delete `available_libraries_parse_successfully` test that calls
  `extract_blocks` — replace with one that parses directly

**Other test files** that use the import syntax in HCL strings.

### 10. Update built-in library files

`library/wok.hcl` and `library/merge.hcl` — no changes needed. Their `const`
blocks are already single-label and will deserialize into `Runbook.consts`.

### 11. Validation: reject imports/consts in non-library contexts

After step 4, add validation in `parse_runbook_inner()`:
- If `runbook.consts` is non-empty and we're NOT in the library parsing path,
  warn or error (consts are only meaningful in libraries).
- Alternatively, just ignore them — they're harmless if unused.

Actually, this isn't needed. The pipeline already handles it: user files have
imports, libraries have consts. The flow naturally separates them.

## Files changed

| File | Change |
|------|--------|
| `crates/runbook/src/parser.rs` | Add `imports`/`consts` fields to `Runbook` |
| `crates/runbook/src/import.rs` | Refactor types, delete extraction code, rewrite pipeline |
| `crates/runbook/src/import_tests.rs` | Delete extraction tests, update remaining |
| `crates/runbook/src/find.rs` | Use parsed imports instead of `extract_blocks()` |
| `crates/runbook/src/lib.rs` | Update re-exports |
| `crates/cli/src/commands/runbook.rs` | Adapt to new types |
| `crates/runbook/tests/parsing/*.rs` | Update any HCL using import syntax |

## Not changed

- `library/wok.hcl`, `library/merge.hcl` — const syntax is already compatible
- `interpolate_consts()` — stays as-is (text-level template substitution)
- `validate_consts()` — minor signature change only
- `merge_runbook()`, `prefix_names()` — unchanged (operate on Runbook struct)
- `resolve_library()`, `available_libraries()` — unchanged
