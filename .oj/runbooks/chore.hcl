# File a chore and dispatch it to a worker.
#
# Worker pulls chores from wok, completes them, and submits to the merge queue.
#
# Examples:
#   oj run chore "Update dependencies to latest versions"
#   oj run chore "Add missing test coverage for auth module"

command "chore" {
  args = "<description>"
  run  = <<-SHELL
    wok new chore "${args.description}"
    oj worker start chore
  SHELL
}

queue "chores" {
  type = "external"
  list = "wok list -t chore -s todo --unassigned -p oj -o json"
  take = "wok start ${item.id}"
}

worker "chore" {
  source      = { queue = "chores" }
  handler     = { pipeline = "chore" }
  concurrency = 3
}

pipeline "chore" {
  name      = "${var.task.title}"
  vars      = ["task"]
  on_cancel = { step = "cancel" }
  on_fail   = { step = "reopen" }

  workspace {
    git    = "worktree"
    branch = "chore/${var.task.id}-${workspace.nonce}"
  }

  locals {
    base   = "main"
    title  = "$(printf 'chore: %.73s' \"${var.task.title}\")"
  }

  notify {
    on_start = "Chore: ${var.task.title}"
    on_done  = "Chore done: ${var.task.title}"
    on_fail  = "Chore failed: ${var.task.title}"
  }

  step "work" {
    run     = { agent = "chores" }
    on_done = { step = "submit" }
  }

  # TODO: hook into merge pipeline to mark issue done instead
  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      test "$(git rev-list --count HEAD ^origin/${local.base})" -gt 0 || { echo "No changes to submit" >&2; exit 1; }
      git push origin "${workspace.branch}"
      cd ${invoke.dir} && wok done ${var.task.id}
      oj queue push merges --var branch="${workspace.branch}" --var title="${local.title}"
    SHELL
  }

  step "reopen" {
    run = "cd ${invoke.dir} && wok reopen ${var.task.id} --reason 'Chore pipeline failed'"
  }

  step "cancel" {
    run = "cd ${invoke.dir} && wok close ${var.task.id} --reason 'Chore pipeline cancelled'"
  }
}

agent "chores" {
  # NOTE: Since chores should quick and small, prevent unnecessary EnterPlanMode and ExitPlanMode
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = { action = "nudge", message = "Keep working. Complete the task, write tests, run make check, and commit." }
  on_dead  = { action = "gate", run = "make check" }

  prime = ["cd ${invoke.dir} && wok show ${var.task.id}"]

  prompt = <<-PROMPT
    Complete the following task: ${var.task.id} - ${var.task.title}

    ## Steps

    1. Understand the task
    2. Find the relevant code
    3. Implement the changes
    4. Write or update tests
    5. Run `make check` to verify
    6. Commit your changes
  PROMPT
}
