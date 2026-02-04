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
  workspace = "ephemeral"
  on_cancel = { step = "cancel" }
  on_fail   = { step = "reopen" }

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "feature/${var.name}-${workspace.nonce}"
    title  = "feat(${var.name}): ${var.instructions}"
  }

  notify {
    on_start = "Building: ${var.name}"
    on_done  = "Build landed: ${var.name}"
    on_fail  = "Build failed: ${var.name}"
  }

  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" HEAD
      mkdir -p plans
      cd ${invoke.dir} && wok new feature "${var.instructions}" -o id > "${workspace.root}/.feature-id"
    SHELL
    on_done = { step = "plan" }
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

  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      test "$(git rev-list --count HEAD ^origin/${var.base})" -gt 0 || { echo "No changes to submit" >&2; exit 1; }
      git -C "${local.repo}" push origin "${local.branch}"
      oj queue push merges --var branch="${local.branch}" --var title="${local.title}"
    SHELL
    on_done = { step = "done" }
  }

  step "done" {
    run     = "cd ${invoke.dir} && wok done $(cat ${workspace.root}/.feature-id)"
    on_done = { step = "cleanup" }
  }

  step "cancel" {
    run     = "cd ${invoke.dir} && wok close $(cat ${workspace.root}/.feature-id) --reason 'Build pipeline cancelled'"
    on_done = { step = "abandon" }
  }

  step "reopen" {
    run     = "cd ${invoke.dir} && wok reopen $(cat ${workspace.root}/.feature-id) --reason 'Build pipeline failed'"
    on_done = { step = "abandon" }
  }

  step "abandon" {
    run = <<-SHELL
      git -C "${local.repo}" worktree remove --force "${workspace.root}" 2>/dev/null || true
      git -C "${local.repo}" branch -D "${local.branch}" 2>/dev/null || true
    SHELL
  }

  step "cleanup" {
    run = "git -C \"${local.repo}\" worktree remove --force \"${workspace.root}\" 2>/dev/null || true"
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
