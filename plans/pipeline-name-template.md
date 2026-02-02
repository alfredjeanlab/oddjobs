# Pipeline Name Template

## Overview

Add an optional `name` template field to pipeline definitions that produces human-readable pipeline names. When set, the template is interpolated with pipeline vars, slugified (lowercased, non-alphanum replaced with hyphens, stop words removed, truncated to 24 chars), and suffixed with a nonce from the pipeline UUID. When unset, existing naming behavior is preserved.

## Project Structure

Key files to modify:

```
crates/runbook/src/pipeline.rs       # Add `name_template` field to PipelineDef
crates/runbook/src/pipeline_tests.rs # Test deserialization of name_template
crates/runbook/src/slug.rs           # NEW: slugify function + stop word list
crates/runbook/src/slug_tests.rs     # NEW: unit tests for slugify
crates/runbook/src/lib.rs            # Re-export slugify
crates/engine/src/runtime/handlers/
  pipeline_create.rs                 # Apply name template during pipeline creation
  command.rs                         # Fall back to template-derived name
  worker.rs                          # Fall back to template-derived name
.oj/runbooks/build.hcl               # Add name template
.oj/runbooks/bug.hcl                 # Add name template
.oj/runbooks/chore.hcl               # Add name template
.oj/runbooks/merge.hcl               # Add name template
```

## Dependencies

No new external dependencies. Uses existing `regex` crate and `oj_runbook::interpolate`.

## Implementation Phases

### Phase 1: Add `name_template` field to `PipelineDef`

**File:** `crates/runbook/src/pipeline.rs`

Add an optional `name_template` field to `PipelineDef`:

```rust
/// Optional name template for human-readable pipeline names.
/// Supports `${var.*}` interpolation. The result is slugified and
/// suffixed with a nonce derived from the pipeline UUID.
#[serde(default)]
pub name_template: Option<String>,
```

The HCL field name is `name` on the pipeline block. However, `PipelineDef` already has a `name` field that stores the pipeline kind (injected from the HCL block label, e.g. `pipeline "build"` → `name = "build"`). To avoid collision, use `name_template` as the Rust field name with `#[serde(alias = "name_template")]` — but actually, looking at the code, the existing `name` field is set by post-processing in the runbook parser (injected from map key), not deserialized from HCL content. So the HCL `name = "..."` attribute can deserialize directly into a new field.

**Decision:** Rename the existing `PipelineDef::name` field to `kind` (it holds the pipeline kind, not a display name), and add a new `name` field for the template. This is cleaner but requires updating all references to `pipeline_def.name` across the codebase.

**Alternative (lower risk):** Keep the existing `name` field as-is and add `name_template` as both the Rust field name and HCL attribute name. The HCL usage would be `name_template = "${var.bug.title}"`. This avoids a rename but the HCL attribute is longer.

**Recommended approach:** Use `name_template` as the Rust field and HCL attribute. It's unambiguous and avoids a risky rename of the existing `name` field.

```rust
#[serde(default)]
pub name_template: Option<String>,
```

HCL usage: `name_template = "${var.bug.title}"`

Wait — the instructions say the HCL field should be `name`. Looking more carefully at the existing code: `PipelineDef::name` is `#[serde(default)]` and is populated by the runbook parser from the HCL block label (`pipeline "build" { }` → name = "build"). If HCL also has `name = "..."` inside the block, serde would deserialize it into the same field, overwriting the block label.

**Resolution:** The instructions say `name = "${var.bug.title}"`. To support this cleanly:
1. Add a new field `name_template: Option<String>` with `#[serde(default, alias = "name")]` — but this conflicts with the existing `name` field.
2. Better: The existing `name` field on `PipelineDef` is injected post-deserialization from the map key (same pattern as `StepDef::name`). If we look at how pipelines are deserialized in the runbook parser, the block label goes into the map key, and the `name` field inside the struct starts empty (default). The runbook parser then sets `pipeline.name = key`. If we put `name = "${var.bug.title}"` in HCL, serde would deserialize it into `PipelineDef::name`, and then the parser would overwrite it with the block label.

**Final approach:** Rename the struct field to avoid the collision:
- Rename `PipelineDef::name` → `PipelineDef::kind` (with `#[serde(default)]`). Update the runbook parser to set `.kind = key` instead of `.name = key`.
- Add `PipelineDef::name` as `Option<String>` with `#[serde(default)]` for the template. This is what gets deserialized from `name = "..."` in HCL.
- Update all code that reads `pipeline_def.name` (for the kind) to read `pipeline_def.kind`.

This is the cleanest approach and matches the instructions' HCL syntax exactly.

**Milestone:** `cargo check` passes with the new field; existing tests still pass.

### Phase 2: Add `slugify` function

**File:** `crates/runbook/src/slug.rs` (new module)

Create a `slugify` function that:
1. Lowercases the input
2. Replaces non-alphanumeric characters with hyphens
3. Collapses multiple consecutive hyphens
4. Removes stop words (words that are entirely a stop word between hyphens)
5. Truncates to 24 characters
6. Trims trailing hyphens

Stop word list (from instructions):
```rust
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
    "have", "has", "had", "do", "does", "did", "will", "would", "shall",
    "should", "may", "might", "must", "can", "could", "to", "of", "in",
    "for", "on", "with", "at", "by", "from", "as", "into", "through",
    "during", "before", "after", "above", "below", "between", "out", "off",
    "over", "under", "again", "further", "then", "once", "that", "this",
    "these", "those", "and", "but", "or", "nor", "not", "so", "yet",
    "both", "each", "every", "all", "any", "few", "more", "most", "other",
    "some", "such", "no", "only", "own", "same", "than", "too", "very",
    "just", "about", "also", "its", "it", "we", "our", "currently",
    "when", "which", "what",
];
```

Function signature:
```rust
/// Slugify a string for use as a pipeline name component.
///
/// Lowercases, replaces non-alphanumeric with hyphens, removes stop words,
/// collapses hyphens, and truncates to `max_len` characters (trimming trailing hyphens).
pub fn slugify(input: &str, max_len: usize) -> String
```

A companion function to build the full pipeline name:
```rust
/// Build a pipeline name from a template result and nonce.
///
/// Slugifies the input, truncates to 24 chars, and appends `-{nonce}`.
pub fn pipeline_display_name(raw: &str, nonce: &str) -> String {
    let slug = slugify(raw, 24);
    if slug.is_empty() {
        nonce.to_string()
    } else {
        format!("{}-{}", slug, nonce)
    }
}
```

Register the module in `crates/runbook/src/lib.rs` and export `slugify` and `pipeline_display_name`.

**Milestone:** Unit tests for slugify pass.

### Phase 3: Apply name template during pipeline creation

**File:** `crates/engine/src/runtime/handlers/pipeline_create.rs`

In `create_and_start_pipeline`, after vars are populated but before the workspace is created, check if `pipeline_def.name` (the template) is set:

```rust
// Resolve pipeline display name from template (if set)
let pipeline_name = if let Some(name_template) = &pipeline_def.name {
    // Build lookup map with var.* prefixes for interpolation
    let lookup: HashMap<String, String> = vars
        .iter()
        .flat_map(|(k, v)| {
            vec![
                (k.clone(), v.clone()),
                (format!("var.{}", k), v.clone()),
            ]
        })
        .collect();
    let raw = oj_runbook::interpolate(name_template, &lookup);
    let nonce = &pipeline_id_str[..8.min(pipeline_id_str.len())];
    oj_runbook::pipeline_display_name(&raw, nonce)
} else {
    pipeline_name // use the name passed in from caller
};
```

This means the `pipeline_name` parameter in `CreatePipelineParams` becomes a fallback. If the template produces a name, it overrides the caller-provided name.

**Files also updated:**
- `command.rs`: Simplify — always pass `pipeline_id.to_string()` as the fallback name (or keep existing logic; the template overrides anyway).
- `worker.rs`: Same — the `format!("{}-{}", pipeline_kind, item_id)` fallback remains but is overridden when template is set.

Actually, the cleaner approach: move name computation entirely into `create_and_start_pipeline`. The callers pass a `fallback_name` and the function uses the template if available:

```rust
// In create_and_start_pipeline, early:
let pipeline_id_str = pipeline_id.as_str();
let nonce = &pipeline_id_str[..8.min(pipeline_id_str.len())];

let pipeline_name = if let Some(name_template) = &pipeline_def.name {
    let lookup: HashMap<String, String> = vars
        .iter()
        .flat_map(|(k, v)| vec![(k.clone(), v.clone()), (format!("var.{}", k), v.clone())])
        .collect();
    let raw = oj_runbook::interpolate(name_template, &lookup);
    oj_runbook::pipeline_display_name(&raw, nonce)
} else {
    pipeline_name  // from CreatePipelineParams
};
```

**Milestone:** Running a pipeline with `name = "${var.bug.title}"` produces a slugified name like `fix-login-button-a1b2c3d4`.

### Phase 4: Update runbook parser for `kind` rename

**Files:** Search for all references to `PipelineDef::name` used as the pipeline kind and update to `.kind`.

Key locations (find with `grep -r "pipeline_def.name\|\.name\.clone\|pipeline\.name" crates/`):
- Runbook parser where block label is injected into `name` → change to `kind`
- Any code reading `pipeline_def.name` for the kind string
- `CreatePipelineParams::pipeline_kind` already exists and is the right field — callers already pass the kind separately

This phase is about ensuring the rename doesn't break anything. The `pipeline_def.name` field was mostly used internally in the runbook parser; external code uses `pipeline_kind` from `CreatePipelineParams`.

**Milestone:** `cargo test --all` passes; `cargo clippy` clean.

### Phase 5: Update runbooks to use name templates

**Files:** `.oj/runbooks/*.hcl`

Add `name` field to each pipeline block:

**build.hcl:**
```hcl
pipeline "build" {
  name = "${var.name}"
  vars = ["name", "instructions", "base", "rebase", "new"]
  ...
}
```

**bug.hcl:**
```hcl
pipeline "fix" {
  name = "${var.bug.title}"
  vars = ["bug"]
  ...
}
```

**chore.hcl:**
```hcl
pipeline "chore" {
  name = "${var.task.title}"
  vars = ["task"]
  ...
}
```

**merge.hcl:**
```hcl
pipeline "merge" {
  name = "${var.mr.branch}"
  vars = ["mr"]
  ...
}
```

**Milestone:** `oj run build test-feature "test"` creates a pipeline named `test-feature-a1b2c3d4` instead of a UUID.

### Phase 6: Add unit tests

**File:** `crates/runbook/src/slug_tests.rs` (new)

Test cases for `slugify`:
- Basic: `"Hello World"` → `"hello-world"`
- Stop words removed: `"Fix the login button"` → `"fix-login-button"`
- Non-alphanum replaced: `"fix: login_button!"` → `"fix-login-button"`
- Multiple hyphens collapsed: `"a---b"` → `"a-b"`
- Truncation: 30-char input → 24 chars, no trailing hyphen
- Empty after stop word removal → empty string
- Already clean slug passes through
- Unicode/special chars handled

Test cases for `pipeline_display_name`:
- Normal: `"fix-login-button"` + `"a1b2c3d4"` → `"fix-login-button-a1b2c3d4"`
- Empty slug: `""` + `"a1b2c3d4"` → `"a1b2c3d4"`
- Truncation boundary: slug exactly 24 chars + nonce

Test cases for name template integration (in `pipeline_create.rs` or as an integration test):
- Pipeline with `name_template` set produces expected name
- Pipeline without `name_template` uses fallback name
- Template with vars that contain special characters

**Milestone:** `cargo test --all` passes with new tests.

## Key Implementation Details

### Slugify algorithm (ordered steps)

1. Lowercase the entire string
2. Replace any run of non-`[a-z0-9]` characters with a single hyphen
3. Split on hyphens, filter out stop words, rejoin with hyphens
4. Trim leading/trailing hyphens
5. Truncate to `max_len` characters
6. Trim trailing hyphens (truncation may leave one)

### Name template resolution timing

The name template must be resolved **before** workspace creation because the workspace ID incorporates the pipeline name (`ws-{name}-{nonce}`). The var lookup at resolution time includes raw vars and `var.*`-prefixed vars, but NOT `local.*` or `workspace.*` (those aren't computed yet). This is intentional — the name is computed from input vars only.

### Interaction with existing `name` field

The existing `PipelineDef::name` field stores the pipeline kind (from the HCL block label). Renaming it to `kind` is necessary to free up `name` for the template. This rename is safe because:
- External code uses `CreatePipelineParams::pipeline_kind`, not `pipeline_def.name`
- The runbook parser injects the block label; just change the target field
- Tests reference the field by name; straightforward find-and-replace

### No-template fallback

When `name_template` is `None`:
- Command handler: uses `args["name"]` if present, else `pipeline_id.to_string()` (existing behavior)
- Worker handler: uses `"{kind}-{item_id}"` (existing behavior)

## Verification Plan

1. **Unit tests** (`cargo test -p oj-runbook`):
   - `slugify` edge cases (empty, all stop words, long input, special chars)
   - `pipeline_display_name` composition
   - `PipelineDef` deserialization with and without `name` field

2. **Integration** (`cargo test --all`):
   - Pipeline creation with name template produces expected name
   - Pipeline creation without name template preserves existing behavior
   - Workspace ID uses the template-derived name

3. **Lint and build**:
   - `cargo fmt --all -- --check`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo build --all`

4. **Manual verification**:
   - `oj run build test-feature "test"` → pipeline named `test-feature-XXXXXXXX`
   - `oj pipeline list` shows human-readable names
