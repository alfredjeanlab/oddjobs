# Epic Command

## Overview

Add an `oj run epic` command that decomposes a feature into wok issues (a root feature + blocking tasks), then builds all tasks sequentially. Unlike `oj run build` which follows plan→implement, this pipeline follows decompose→build where the decompose agent creates structured work items and the build agent iterates through them.

The entire implementation is a single new runbook file: `.oj/runbooks/epic.hcl`.

## Project Structure

```
.oj/runbooks/
├── build.hcl     # Existing: plan → implement → submit
├── bug.hcl       # Existing: queue-based fix pipeline
├── chore.hcl     # Existing: queue-based chore pipeline
├── draft.hcl     # Existing: draft branches
├── merge.hcl     # Existing: merge queue
└── epic.hcl      # NEW: decompose → build → submit
```

Single file to create: `.oj/runbooks/epic.hcl`

Contains:
- `command "epic"` — CLI entry point
- `pipeline "epic"` — 5 steps: init, decompose, build, submit, cleanup
- `agent "decompose"` — explores codebase and creates wok issues
- `agent "epic-builder"` — works through tasks until all are done

## Dependencies

No new code dependencies. Uses existing infrastructure:

- **wok CLI** — issue tracker (`wok new feature`, `wok new task --blocks`, `wok dep`, `wok prime`, `wok tree`, `wok start`, `wok done`, `wok ready`, `wok show`)
- **Existing merge queue** — `oj queue push merges` from merge.hcl
- **Existing pipeline primitives** — ephemeral workspace, agent lifecycle, gate actions

## Implementation Phases

### Phase 1: Command and pipeline skeleton

Create `.oj/runbooks/epic.hcl` with the command definition, pipeline structure, and shell steps (init, submit, cleanup). These follow established patterns from `build.hcl`.

```hcl
# Decompose work into issues and build them all.
#
# Creates a feature issue with blocking tasks, then implements each task
# sequentially until the epic is complete.
#
# Examples:
#   oj run epic auth "Add user authentication with JWT tokens"
#   oj run epic dark-mode "Implement dark mode" --blocked-by 42,43

command "epic" {
  args = "<name> <instructions> [--blocked-by <ids>]"
  run  = { pipeline = "epic" }

  defaults = {
    blocked-by = ""
  }
}

pipeline "epic" {
  name      = "${var.name}"
  vars      = ["name", "instructions", "blocked-by"]
  workspace = "ephemeral"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "feature/${var.name}-${workspace.nonce}"
    title  = "feat(${var.name}): ${var.instructions}"
  }

  notify {
    on_start = "Epic started: ${var.name}"
    on_done  = "Epic landed: ${var.name}"
    on_fail  = "Epic failed: ${var.name}"
  }

  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" HEAD
    SHELL
    on_done = { step = "decompose" }
  }

  step "decompose" {
    run     = { agent = "decompose" }
    on_done = { step = "build" }
  }

  step "build" {
    run     = { agent = "epic-builder" }
    on_done = { step = "submit" }
  }

  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      test "$(git rev-list --count HEAD ^origin/main)" -gt 0 || { echo "No changes to submit" >&2; exit 1; }
      git -C "${local.repo}" push origin "${local.branch}"
      oj queue push merges --var branch="${local.branch}" --var title="${local.title}"
    SHELL
    on_done = { step = "cleanup" }
  }

  step "cleanup" {
    run = "git -C \"${local.repo}\" worktree remove --force \"${workspace.root}\" 2>/dev/null || true"
  }
}
```

**Key decisions:**
- Branch pattern: `feature/${var.name}-${workspace.nonce}` (matches build.hcl)
- Submit step: identical to build.hcl, hardcoded to `main` base (no `--base` arg on epic)
- No `on_fail`/`on_cancel` pipeline handlers — on failure, workspace is cleaned up normally; wok issues remain in their current state for manual triage

### Phase 2: Decompose agent

The decompose agent explores the codebase and creates structured wok issues. It uses prime commands to inject project context and a gate to verify the root issue ID was written.

```hcl
agent "decompose" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = { action = "gate", run = "test -s .epic-root-id" }
  on_dead  = "fail"

  prime = [
    "wok prime",
    "echo '## Ready Issues'",
    "wok ready",
    "echo '## Project Instructions'",
    "cat CLAUDE.md 2>/dev/null || true",
  ]

  prompt = <<-PROMPT
    You are decomposing a feature epic into concrete, implementable tasks.

    ## Epic

    **Name:** ${var.name}
    **Instructions:** ${var.instructions}

    ## Process

    1. **Explore the codebase thoroughly** — launch 3-5 Explore subagents to understand
       different aspects: architecture, existing patterns, relevant modules, test patterns,
       and dependencies related to this work.

    2. **Clarify requirements** — if the instructions are ambiguous or you need to make
       significant design decisions, use AskUserQuestion to ask the user before proceeding.

    3. **Create the root feature issue:**
       ```
       wok new feature '${var.name}'
       ```
       Note the returned issue ID (the root ID).

    4. **Create blocking task issues** (aim for 3-8 tasks):
       ```
       wok new task '<clear description with acceptance criteria>' --blocks <root-id>
       ```
       Each task should be independently implementable and testable.

    5. **Write the root issue ID** to `.epic-root-id`:
       ```
       echo <root-id> > .epic-root-id
       ```

    6. **Handle dependencies** — if blocked-by IDs were provided ("${var.blocked-by}"),
       run `wok dep <root-id> <id>` for each ID in the comma-separated list.

    ## Task Decomposition Guidelines

    - Each task should be a single, focused unit of work
    - Include clear acceptance criteria in the task description
    - Order tasks so dependencies flow naturally (earlier tasks set up what later ones need)
    - Typical epic has 3-8 tasks — enough granularity for progress tracking without
      over-decomposition
    - Consider: data model changes first, then core logic, then integration, then tests/polish

    When done, say "I'm done" and wait.
  PROMPT
}
```

**Key decisions:**
- `--disallowed-tools ExitPlanMode,EnterPlanMode` — decompose creates issues, not plan files
- `on_idle = gate` with `test -s .epic-root-id` — verifies the root ID file exists and is non-empty before advancing; if the agent goes idle without writing it, the gate fails and escalates
- `on_dead = "fail"` — if the agent dies during decomposition, fail the pipeline (decomposition is critical and non-recoverable without context)
- Prime commands inject `wok prime` (project onboarding), `wok ready` (awareness of existing work), and `CLAUDE.md` (project conventions)
- Prompt encourages 3-5 Explore subagents for thorough codebase understanding
- Prompt encourages AskUserQuestion for ambiguous requirements
- The `--blocked-by` handling is in the prompt text — the agent reads the value and runs `wok dep` commands if non-empty

### Phase 3: Build agent

The build agent is long-lived, working through all tasks in the epic. It uses `recover` on death (so it can resume after crashes) and a gate on idle that checks whether all tasks are complete.

```hcl
agent "epic-builder" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = { action = "gate", run = "root_id=$(cat .epic-root-id) && ! wok tree \"$root_id\" | grep -qE '(todo|doing)'", attempts = "forever" }
  on_dead  = { action = "recover", append = true, message = "Continue working on the epic. Check `wok tree $(cat .epic-root-id)` for remaining tasks." }

  prime = [
    "wok prime $(cat .epic-root-id)",
    "echo '## Epic Tree'",
    "wok tree $(cat .epic-root-id)",
    "echo '## Root Issue'",
    "wok show $(cat .epic-root-id)",
  ]

  prompt = <<-PROMPT
    You are working through an epic, implementing each task until all are complete.

    ## Workflow

    1. Check the current state: `wok tree $(cat .epic-root-id)`
    2. Pick the next unblocked task with status "todo"
    3. Start it: `wok start <task-id>`
    4. Implement the task — write code, write/update tests
    5. Verify: `make check`
    6. Commit changes: `git add -A && git commit -m 'feat(${var.name}): <brief description>'`
    7. Mark done: `wok done <task-id>`
    8. Repeat from step 1 until all tasks are complete

    ## Guidelines

    - **Commit after each task** — not just at the end
    - **Run `make check` before marking done** — ensure nothing is broken
    - **Skip blocked tasks** — if a task depends on another that isn't done, work on a different one
    - **Use conventional commits** — `feat(${var.name}): <description>` for features, `test(${var.name}): ...` for test-only changes
    - **Check progress regularly** — `wok tree $(cat .epic-root-id)` shows the full status
    - When all tasks show as done, say "I'm done"
  PROMPT
}
```

**Key decisions:**

- **`on_idle` gate with `attempts = "forever"`:** The gate checks if any tasks still have `todo` or `doing` status in the wok tree. If tasks remain (grep finds matches), the negation makes exit code 1, and `attempts = "forever"` causes a nudge instead of escalation. If all tasks are done (grep finds nothing), exit code 0 advances the pipeline to submit.

- **`on_dead = recover` with `append = true`:** If the agent process dies (Claude crashes, OOM, etc.), it's automatically restarted with the recovery message appended to context. This is critical because the build step is long-lived and may take many iterations. The agent picks up where it left off by checking `wok tree`.

- **`--disallowed-tools ExitPlanMode,EnterPlanMode`:** The builder should implement directly, not create plans.

- **Prime commands with `$(cat .epic-root-id)`:** The root ID is read from the file written by the decompose step. Prime injects the wok context (onboarding for this specific epic), the task tree, and the root issue details.

- **Commit-per-task pattern:** The prompt instructs committing after each task. This means the submit step only needs to handle any final uncommitted changes and the push/merge-queue submission.

### Phase 4: Verification

1. **Parse validation:** Run `ojd` or use the runbook parser to verify `epic.hcl` parses without errors. Check that all template variables resolve correctly and agent names don't conflict with existing runbooks.

2. **Dry run — small epic:** Test with a small, well-defined epic:
   ```
   oj run epic test-epic "Add a hello world endpoint that returns 200 OK"
   ```
   Verify:
   - The decompose agent creates a root feature and 2-3 tasks in wok
   - The `.epic-root-id` file is written
   - The gate check advances to the build step
   - The build agent picks up tasks, implements, commits per task
   - The submit step pushes and queues for merge

3. **Edge cases to verify:**
   - `--blocked-by` with multiple IDs: `oj run epic foo "..." --blocked-by 10,11,12`
   - `--blocked-by` omitted (default empty string)
   - Build agent recovery: kill the agent process mid-task and verify it recovers
   - Gate semantics: verify the on_idle gate nudges (not escalates) when tasks remain

## Key Implementation Details

### Agent naming
Existing agents across all runbook files: `plan`, `implement`, `bugs`, `chores`, `resolver`, `refiner`, `draft-resolver`. The new agents are named `decompose` and `epic-builder` to avoid conflicts.

### Gate check for build completion
The build agent's on_idle gate runs:
```bash
root_id=$(cat .epic-root-id) && ! wok tree "$root_id" | grep -qE '(todo|doing)'
```
- Reads the root ID from the file written by the decompose step
- Pipes `wok tree` output through grep looking for incomplete statuses
- The `!` negation means: no incomplete tasks → exit 0 (advance), incomplete tasks exist → exit 1
- `attempts = "forever"` ensures the agent is nudged (not escalated) when tasks remain

### Root ID file as inter-step communication
The decompose step writes the wok root issue ID to `${workspace.root}/.epic-root-id`. The build step reads it via `$(cat .epic-root-id)` in prime commands, gate checks, and prompt instructions. This avoids parsing agent output and works reliably across agent restarts (recover).

### The blocked-by argument
The `--blocked-by` arg is optional (defaults to `""`). It's passed through to the decompose agent's prompt as `${var.blocked-by}`. The agent checks if it's non-empty and runs `wok dep <root-id> <id>` for each comma-separated ID. This keeps the pipeline simple — no shell parsing of the arg in pipeline steps.

## Verification Plan

1. **Syntax check:** Ensure `epic.hcl` is valid HCL and passes runbook validation (no duplicate entity names, all template variables resolve, all referenced agents exist within the file)

2. **Unit test scope:** No new Rust code, so no unit tests needed. The runbook parser's existing tests cover HCL parsing. If desired, add a parser test that loads `epic.hcl` and asserts it produces the expected command/pipeline/agent structures.

3. **Integration test:** Run `oj run epic` end-to-end on a test project with wok configured. Verify the full lifecycle: init → decompose (creates issues) → build (implements all) → submit (pushes to merge queue) → cleanup.

4. **Checklist:**
   - [ ] `epic.hcl` parses without errors
   - [ ] `oj run epic --help` shows correct arg spec
   - [ ] Decompose agent creates root feature + blocking tasks
   - [ ] `.epic-root-id` file is written and gate advances
   - [ ] Build agent works through tasks with commit-per-task
   - [ ] Build gate advances when all tasks are done
   - [ ] Submit step pushes branch and queues for merge
   - [ ] `--blocked-by` creates wok dependencies when provided
   - [ ] `--blocked-by` omission works (empty default)
   - [ ] Build agent recovers from unexpected death
