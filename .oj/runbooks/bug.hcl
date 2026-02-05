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
  list = "wok ready -t bug -p oj -o json"
  take = "wok start ${item.id}"
}

worker "bug" {
  source      = { queue = "bugs" }
  handler     = { job = "bug" }
  concurrency = 3
}

job "bug" {
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

  # TODO: hook into merge job to mark issue done instead
  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      if test "$(git rev-list --count HEAD ^origin/${local.base})" -gt 0; then
        git push origin "${workspace.branch}"
        wok done ${var.bug.id}
        oj queue push merges --var branch="${workspace.branch}" --var title="${local.title}"
      elif wok show ${var.bug.id} -o json | grep -q '"status":"done"'; then
        echo "Issue already resolved, no changes needed"
      else
        echo "No changes to submit" >&2
        exit 1
      fi
    SHELL
  }

  step "reopen" {
    run = "wok reopen ${var.bug.id} --reason 'Fix job failed'"
  }

  step "cancel" {
    run = "wok close ${var.bug.id} --reason 'Fix job cancelled'"
  }
}

agent "bugs" {
  # NOTE: Since bugs should quick and small, prevent unnecessary EnterPlanMode and ExitPlanMode
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = { action = "nudge", message = "Keep working. Fix the bug, write tests, run make check, and commit." }
  on_dead  = { action = "gate", run = "make check" }

  session "tmux" {
    color = "blue"
    title = "Bug: ${var.bug.id}"
    status {
      left  = "${var.bug.id}: ${var.bug.title}"
      right = "${workspace.branch}"
    }
  }

  prime = ["wok show ${var.bug.id}"]

  prompt = <<-PROMPT
    Fix the following bug: ${var.bug.id} - ${var.bug.title}

    ## Steps

    1. Understand the bug
    2. Find the relevant code
    3. Implement a fix
    4. Write or update tests
    5. Run `make check` to verify
    6. Commit your changes
    7. Mark the issue as done: `wok done ${var.bug.id}`

    If the bug is already fixed (e.g. by a prior commit), skip to step 7.
  PROMPT
}
