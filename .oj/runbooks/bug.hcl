# File a bug and dispatch it to a fix worker.
#
# Worker pulls bugs from wok, fixes them, and submits to the merge queue.
#
# Examples:
#   oj run fix "Button doesn't respond to clicks"
#   oj run fix "Login page crashes on empty password"

command "fix" {
  args = "<description>"
  run  = <<-SHELL
    wok new bug "${args.description}"
    oj worker start bug
  SHELL
}

queue "bugs" {
  type = "external"
  list = "wok list -t bug -s todo --unassigned -p oj -o json"
  take = "wok start ${item.id}"
}

worker "bug" {
  source      = { queue = "bugs" }
  handler     = { pipeline = "bug" }
  concurrency = 3
}

pipeline "bug" {
  name      = "${var.bug.title}"
  vars      = ["bug"]
  on_fail   = { step = "reopen" }
  on_cancel = { step = "cancel" }

  workspace {
    git    = "worktree"
    branch = "fix/${var.bug.id}-${workspace.nonce}"
  }

  locals {
    base   = "main"
    title  = "$(printf 'fix: %.75s' \"${var.bug.title}\")"
  }

  notify {
    on_start = "Fixing: ${var.bug.title}"
    on_done  = "Fix landed: ${var.bug.title}"
    on_fail  = "Fix failed: ${var.bug.title}"
  }

  step "fix" {
    run     = { agent = "bugs" }
    on_done = { step = "submit" }
  }

  # TODO: hook into merge pipeline to mark issue done instead
  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      test "$(git rev-list --count HEAD ^origin/${local.base})" -gt 0 || { echo "No changes to submit" >&2; exit 1; }
      git push origin "${workspace.branch}"
      cd ${invoke.dir} && wok done ${var.bug.id}
      oj queue push merges --var branch="${workspace.branch}" --var title="${local.title}"
    SHELL
  }

  step "reopen" {
    run = "cd ${invoke.dir} && wok reopen ${var.bug.id} --reason 'Fix pipeline failed'"
  }

  step "cancel" {
    run = "cd ${invoke.dir} && wok close ${var.bug.id} --reason 'Fix pipeline cancelled'"
  }
}

agent "bugs" {
  # NOTE: Since bugs should quick and small, prevent unnecessary EnterPlanMode and ExitPlanMode
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = { action = "nudge", message = "Keep working. Fix the bug, write tests, run make check, and commit." }
  on_dead  = { action = "gate", run = "make check" }

  prime = ["cd ${invoke.dir} && wok show ${var.bug.id}"]

  prompt = <<-PROMPT
    Fix the following bug: ${var.bug.id} - ${var.bug.title}

    ## Steps

    1. Understand the bug
    2. Find the relevant code
    3. Implement a fix
    4. Write or update tests
    5. Run `make check` to verify
    6. Commit your changes
  PROMPT
}
