command "epic" {
  args = "<name> <instructions> [--blocked-by <ids>]"
  run  = { job = "epic" }

  defaults = {
    blocked-by = ""
  }
}

job "epic" {
  name      = "${var.name}"
  vars      = ["name", "instructions", "blocked-by"]
  source = "folder"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "feature/${var.name}-${source.nonce}"
    title  = "feat(${var.name}): ${var.instructions}"
  }

  notify {
    on_start = "Epic started: ${var.name}"
    on_done  = "Epic landed: ${var.name}"
    on_fail  = "Epic failed: ${var.name}"
  }

  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree add -b "${local.branch}" "${source.root}" HEAD
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
    run = "git -C \"${local.repo}\" worktree remove --force \"${source.root}\" 2>/dev/null || true"
  }
}

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

  prompt = "Decompose the epic into tasks."
}

agent "epic-builder" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = { action = "gate", run = "root_id=$(cat .epic-root-id) && ! wok tree \"$root_id\" | grep -qE '(todo|doing)'", attempts = "forever" }
  on_dead  = { action = "resume", append = true, message = "Continue working on the epic." }

  prime = [
    "wok prime $(cat .epic-root-id)",
    "echo '## Epic Tree'",
    "wok tree $(cat .epic-root-id)",
    "echo '## Root Issue'",
    "wok show $(cat .epic-root-id)",
  ]

  prompt = "Work through the epic tasks."
}
