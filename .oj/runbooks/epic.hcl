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

  workspace {
    git    = "worktree"
    branch = "feature/${var.name}-${workspace.nonce}"
  }

  locals {
    title  = "$(printf 'feat(${var.name}): %.72s' \"${var.instructions}\")"
  }

  notify {
    on_start = "Epic started: ${var.name}"
    on_done  = "Epic landed: ${var.name}"
    on_fail  = "Epic failed: ${var.name}"
  }

  step "init" {
    run     = "true"
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
      git push origin "${workspace.branch}"
      oj queue push merges --var branch="${workspace.branch}" --var title="${local.title}"
    SHELL
  }
}

# ------------------------------------------------------------------------------
# Agents
# ------------------------------------------------------------------------------

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
