# Plan and implement exploratory work, pushed to a draft branch (no merge).
#
# Drafts are pushed to draft/<name> branches for later review.
# Use 'oj run drafts' to list or close drafts.
#
# Examples:
#   oj run draft inline-commands "Execute shell commands locally instead of via daemon"
#   oj run draft new-parser "Prototype a new TOML-based runbook parser"

command "draft" {
  args = "<name> <instructions> [--base <branch>]"
  run  = { pipeline = "draft" }

  defaults = {
    base = "main"
  }
}

# Rebase a draft branch onto its base, with agent conflict resolution.
#
# Examples:
#   oj run draft-rebase inline-commands
#   oj run draft-rebase inline-commands --base develop
command "draft-rebase" {
  args = "<name> [--base <branch>]"
  run  = { pipeline = "draft-rebase" }

  defaults = {
    base = "main"
  }
}

# Refine an existing draft branch with additional instructions.
#
# Examples:
#   oj run draft-refine inline-commands "Use bash with set -euo pipefail to match engine"
#   oj run draft-refine new-parser "Add error recovery for malformed input"
command "draft-refine" {
  args = "<name> <instructions>"
  run  = { pipeline = "draft-refine" }
}


# List open draft branches, or close one.
#
# Examples:
#   oj run drafts
#   oj run drafts --close inline-commands
command "drafts" {
  args = "[--close <name>]"
  run  = <<-SHELL
    if test -n "${args.close}"; then
      branch=$(git branch -r --list "origin/draft/${args.close}*" | head -1 | tr -d ' ')
      test -n "$branch" || { echo "No draft matching '${args.close}'"; exit 1; }
      short=$(echo "$branch" | sed 's|^origin/||')
      git push origin --delete "$short"
      echo "Closed $short"
    else
      git fetch --prune origin 2>&1 || true
      branches=$(git branch -r --list 'origin/draft/*' | tr -d ' ')
      if test -z "$branches"; then
        echo "  No open drafts"
      else
        echo "$branches" | while read branch; do
          msg=$(git log -1 --format='%s (%ar)' "$branch")
          short=$(echo "$branch" | sed 's|^origin/||')
          echo "  $short — $msg"
        done
      fi
    fi
  SHELL

  defaults = {
    close = ""
  }
}

pipeline "draft" {
  name      = "${var.name}"
  vars      = ["name", "instructions", "base"]

  workspace {
    git    = "worktree"
    branch = "draft/${var.name}-${workspace.nonce}"
  }

  locals {
    title  = "$(printf 'draft(${var.name}): %.71s' \"${var.instructions}\")"
  }

  notify {
    on_start = "Drafting: ${var.name}"
    on_done  = "Draft ready: ${var.name}"
    on_fail  = "Draft failed: ${var.name}"
  }

  step "init" {
    run     = "mkdir -p plans"
    on_done = { step = "plan" }
  }

  step "plan" {
    run     = { agent = "plan" }
    on_done = { step = "implement" }
  }

  step "implement" {
    run     = { agent = "implement" }
    on_done = { step = "push" }
  }

  step "push" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      git push origin "${workspace.branch}"
    SHELL
  }
}

pipeline "draft-rebase" {
  name      = "rebase-${var.name}"
  vars      = ["name", "base"]
  workspace = "folder"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "$(git -C ${invoke.dir} branch -r --list 'origin/draft/${var.name}*' | head -1 | tr -d ' ' | sed 's|^origin/||')"
  }

  on_cancel = { step = "cleanup" }

  notify {
    on_start = "Rebasing draft: ${var.name}"
    on_done  = "Rebased draft: ${var.name}"
    on_fail  = "Rebase failed: ${var.name}"
  }

  step "init" {
    run = <<-SHELL
      test -n "${local.branch}" || { echo "No draft matching '${var.name}'"; exit 1; }
      git -C "${local.repo}" fetch origin ${var.base} ${local.branch}
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" origin/${local.branch}
    SHELL
    on_done = { step = "rebase" }
  }

  step "rebase" {
    run     = "git rebase origin/${var.base}"
    on_done = { step = "push" }
    on_fail = { step = "resolve" }
  }

  step "resolve" {
    run     = { agent = "draft-resolver" }
    on_done = { step = "push" }
  }

  step "push" {
    run = <<-SHELL
      git -C "${local.repo}" push origin ${local.branch} --force-with-lease
    SHELL
    on_done = { step = "cleanup" }
  }

  step "cleanup" {
    run = <<-SHELL
      git -C "${local.repo}" worktree remove --force "${workspace.root}" 2>/dev/null || true
      git -C "${local.repo}" branch -D "${local.branch}" 2>/dev/null || true
    SHELL
  }
}

pipeline "draft-refine" {
  name      = "refine-${var.name}"
  vars      = ["name", "instructions"]
  workspace = "folder"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "$(git -C ${invoke.dir} branch -r --list 'origin/draft/${var.name}*' | head -1 | tr -d ' ' | sed 's|^origin/||')"
    title  = "$(printf 'refine(${var.name}): %.69s' \"${var.instructions}\")"
  }

  on_cancel = { step = "cleanup" }

  notify {
    on_start = "Refining draft: ${var.name}"
    on_done  = "Draft refined: ${var.name}"
    on_fail  = "Refine failed: ${var.name}"
  }

  step "init" {
    run = <<-SHELL
      test -n "${local.branch}" || { echo "No draft matching '${var.name}'"; exit 1; }
      git -C "${local.repo}" fetch origin ${local.branch}
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" origin/${local.branch}
    SHELL
    on_done = { step = "refine" }
  }

  step "refine" {
    run     = { agent = "refiner" }
    on_done = { step = "push" }
  }

  step "push" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      git -C "${local.repo}" push origin ${local.branch} --force-with-lease
    SHELL
    on_done = { step = "cleanup" }
  }

  step "cleanup" {
    run = <<-SHELL
      git -C "${local.repo}" worktree remove --force "${workspace.root}" 2>/dev/null || true
      git -C "${local.repo}" branch -D "${local.branch}" 2>/dev/null || true
    SHELL
  }
}


# ------------------------------------------------------------------------------
# Agents
# ------------------------------------------------------------------------------

agent "plan" {
  run      = "claude --model opus --dangerously-skip-permissions"
  on_idle  = { action = "nudge", message = "Keep working. Write the plan to plans/${var.name}.md and say 'I'm done' when finished." }
  on_dead  = { action = "gate", run = "test -f plans/${var.name}.md" }

  prompt = <<-PROMPT
    Create an implementation plan for the given instructions.

    ## Output

    Write the plan to `plans/${var.name}.md`.

    ## Structure

    1. **Overview** - Brief summary of what will be built
    2. **Project Structure** - Directory layout and key files
    3. **Dependencies** - External libraries or tools needed
    4. **Implementation Phases** - Numbered phases with clear milestones
    5. **Key Implementation Details** - Important algorithms, patterns, or decisions
    6. **Verification Plan** - How to test the implementation

    ## Guidelines

    - Break work into 3-6 phases
    - Each phase should be independently verifiable
    - Include code snippets for complex patterns
    - Reference existing project files when relevant
    - Keep phases focused and achievable

    ## Constraints

    - ONLY write to `plans/${var.name}.md` — do NOT create or modify source files
    - Do not implement anything — a separate agent handles implementation
    - Do not run builds or tests — just produce the plan
    - When you are done, say "I'm done" and wait.

    Instructions:
    ${var.instructions}

    ---

    Plan name: ${var.name}. Write to plans/${var.name}.md
  PROMPT
}

agent "implement" {
  run      = "claude --model opus --dangerously-skip-permissions"
  on_idle  = { action = "nudge", message = "Keep working. Follow the plan in plans/${var.name}.md, implement all phases, run make check, and commit." }
  on_dead  = { action = "gate", run = "make check" }

  prompt = <<-PROMPT
    Implement the plan in `plans/${var.name}.md`.

    This is a DRAFT — exploratory work that won't be merged yet.
    Focus on getting a working implementation, but don't cut corners on tests.

    ## Steps

    1. Read the plan in `plans/${var.name}.md`
    2. Implement all changes described in the plan
    3. Write tests for new functionality
    4. Run `make check` to verify everything passes
    5. Commit your changes

    ## Context

    Feature request (for reference):
    > ${var.instructions}

    Follow the plan carefully. Ensure all phases are completed and tests pass.
  PROMPT
}

agent "refiner" {
  run      = "claude --model opus --dangerously-skip-permissions"
  on_idle  = { action = "nudge", message = "Keep working. Follow the instructions, run make check, and commit." }
  on_dead  = { action = "gate", run = "make check" }

  prompt = <<-PROMPT
    Refine this draft branch with the following changes.

    ## Instructions

    ${var.instructions}

    ## Steps

    1. Read the existing code to understand what's been built
    2. Make the requested changes
    3. Run `make check` to verify everything passes
    4. Commit your changes

    Keep changes focused on the instructions. Don't refactor unrelated code.
  PROMPT
}


agent "draft-resolver" {
  run      = "claude --model opus --dangerously-skip-permissions"
  on_idle  = { action = "gate", run = "make check", attempts = 2 }
  on_dead  = { action = "escalate" }

  prompt = <<-PROMPT
    You are rebasing draft branch ${local.branch} onto ${var.base}.

    The previous step failed -- either a rebase conflict or a test failure.

    1. Run `git status` to check for rebase conflicts
    2. If conflicts exist, resolve them and `git add` the files
    3. If mid-rebase, run `git rebase --continue` to proceed
    4. Run `make check` to verify everything passes
    5. Fix any test failures
    6. When `make check` passes, say "I'm done"
  PROMPT
}
