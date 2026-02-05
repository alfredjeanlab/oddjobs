# Plan and implement a feature using Claude's native planning mode.
#
# Claude will create a plan, request approval, then implement.
#
# Examples:
#   oj run plan auth "Add user authentication with JWT tokens"
#   oj run plan dark-mode "Implement dark mode theme"

command "plan" {
  args = "<name> <instructions>"
  run  = { job = "plan" }
}

job "plan" {
  name = "${var.name}"
  vars = ["name", "instructions"]

  workspace {
    git    = "worktree"
    branch = "feature/${var.name}-${workspace.nonce}"
  }

  locals {
    title = "$(printf 'feat(${var.name}): %.72s' \"${var.instructions}\")"
  }

  notify {
    on_start = "Building: ${var.name}"
    on_done  = "Build landed: ${var.name}"
    on_fail  = "Build failed: ${var.name}"
  }

  step "build" {
    run     = { agent = "claude" }
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

agent "claude" {
  run      = "claude --model opus --permission-mode plan"
  on_idle  = { action = "nudge", message = "Keep working. Implement the feature, run make check, and commit." }
  on_dead  = { action = "gate", run = "make check" }

  prompt = <<-PROMPT
    Implement: ${var.instructions}

    Create a plan, then implement it. Run `make check` to verify everything passes.
  PROMPT
}
