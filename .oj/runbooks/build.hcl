# Plan and implement a feature, then submit to the merge queue.
#
# Prereq: configure sccache in .cargo/config.toml so worktrees get fast
# builds without sharing a target-dir (which causes cache poisoning):
#
#   [build]
#   rustc-wrapper = "sccache"
#
# Examples:
#   oj run build auth "Add user authentication with JWT tokens"
#   oj run build dark-mode "Implement dark mode theme" --base develop

command "build" {
  args = "<name> <instructions> [--base <branch>]"
  run  = { pipeline = "build" }

  defaults = {
    base = "main"
  }
}

pipeline "build" {
  name      = "${var.name}"
  vars      = ["name", "instructions", "base"]
  on_fail   = { step = "reopen" }
  on_cancel = { step = "cancel" }

  workspace {
    git    = "worktree"
    branch = "feature/${var.name}-${workspace.nonce}"
  }

  locals {
    title  = "$(printf 'feat(${var.name}): %.72s' \"${var.instructions}\")"
    issue  = "$(cd ${invoke.dir} && wok new feature \"${var.instructions}\" -o id)"
  }

  notify {
    on_start = "Building: ${var.name}"
    on_done  = "Build landed: ${var.name}"
    on_fail  = "Build failed: ${var.name}"
  }

  # Ask agent to create plan
  step "plan" {
    run     = { agent = "plan" }
    on_done = { step = "implement" }
  }

  # Ask agent to implement plan
  step "implement" {
    run     = { agent = "implement" }
    on_done = { step = "submit" }
  }

  # TODO: hook into merge pipeline to mark issue done instead
  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      test "$(git rev-list --count HEAD ^origin/${var.base})" -gt 0 || { echo "No changes to submit" >&2; exit 1; }
      git push origin "${workspace.branch}"
      cd ${invoke.dir} && wok done ${local.issue}
      oj queue push merges --var branch="${workspace.branch}" --var title="${local.title}"
    SHELL
  }

  step "cancel" {
    run = "cd ${invoke.dir} && wok close ${local.issue} --reason 'Build pipeline cancelled'"
  }

  step "reopen" {
    run = "cd ${invoke.dir} && wok reopen ${local.issue} --reason 'Build pipeline failed'"
  }
}

# ------------------------------------------------------------------------------
# Agents
# ------------------------------------------------------------------------------

agent "plan" {
  run      = "claude --model opus --dangerously-skip-permissions"
  on_idle  = { action = "nudge", message = "Keep working. Write the plan to plans/${var.name}.md." }
  on_dead  = { action = "gate", run = "test -f plans/${var.name}.md" }

  prime = ["cd ${invoke.dir} && wok show ${local.issue}"]

  prompt = <<-PROMPT
    Create an implementation plan for: ${local.issue} - ${var.instructions}

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

  prime = ["cd ${invoke.dir} && wok show ${local.issue}"]

  prompt = <<-PROMPT
    Implement the plan in `plans/${var.name}.md`.

    ## Steps

    1. Read the plan in `plans/${var.name}.md`
    2. Implement all changes described in the plan
    3. Write tests for new functionality
    4. Run `make check` to verify everything passes
    5. Commit your changes

    ## Context

    Issue: ${local.issue}

    Instructions:

    ${var.instructions}

    Follow the plan carefully. Ensure all phases are completed and tests pass.
  PROMPT
}
