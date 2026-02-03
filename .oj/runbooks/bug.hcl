# Bugfix Runbook
#
# Worker pool: worker pulls bugs from wok → fix → verify → push
#
# Usage:
#   oj run fix <description>        # File a bug and start the worker
#
# Usage (Manual):
#   wok new bug "description"       # File a bug manually
#   oj worker start fix             # Start the worker

command "fix" {
  args = "<description>"
  run  = <<-SHELL
    wok new bug "${args.description}"
    oj worker start fix
  SHELL
}

queue "bugs" {
  type = "external"
  list = "wok list -t bug -s todo --unassigned -o json"
  take = "wok start ${item.id}"
}

worker "fix" {
  source      = { queue = "bugs" }
  handler     = { pipeline = "fix" }
  concurrency = 3
}

pipeline "fix" {
  name      = "${var.bug.title}"
  vars      = ["bug"]
  workspace = "ephemeral"
  on_cancel = { step = "cancel" }
  on_fail   = { step = "reopen" }

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "fix/${var.bug.id}-${workspace.nonce}"
    title  = "fix: ${var.bug.title}"
  }

  notify {
    on_start = "Fixing: ${var.bug.title}"
    on_done  = "Fix landed: ${var.bug.title}"
    on_fail  = "Fix failed: ${var.bug.title}"
  }

  # Initialize workspace: worktree with shared build cache via .cargo/config.toml
  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" HEAD
      mkdir -p .cargo
      echo "[build]" > .cargo/config.toml
      echo "target-dir = \"${local.repo}/target\"" >> .cargo/config.toml
    SHELL
    on_done = { step = "fix" }
  }

  step "fix" {
    run     = { agent = "bugs" }
    on_done = { step = "submit" }
  }

  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      git -C "${local.repo}" push origin "${local.branch}"
      oj queue push merges --var branch="${local.branch}" --var title="${local.title}"
    SHELL
    on_done = { step = "done" }
  }

  step "done" {
    run = "cd ${invoke.dir} && wok done ${var.bug.id}"
  }

  step "cancel" {
    run = "cd ${invoke.dir} && wok close ${var.bug.id} --reason 'Fix pipeline cancelled'"
  }

  step "reopen" {
    run = "cd ${invoke.dir} && wok reopen ${var.bug.id} --reason 'Fix pipeline failed'"
  }
}

agent "bugs" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "nudge", message = "Keep working. Fix the bug, write tests, run make check, and commit." }
  on_dead  = { action = "gate", run = "make check" }

  prompt = <<-PROMPT
    Fix the following bug:

    ${var.bug.title}

    ## Steps

    1. Understand the bug
    2. Find the relevant code
    3. Implement a fix
    4. Write or update tests
    5. Run `make check` to verify
    6. Commit your changes
  PROMPT
}
