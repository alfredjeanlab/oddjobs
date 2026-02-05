# Wok-based issue queues: fix bugs and complete chores.
#
# Consts:
#   prefix - wok label prefix (required)
#   check  - verification command (default: "true")

const "prefix" {}
const "check" { default = "true" }

# File a wok bug and dispatch it to a fix worker.
command "fix" {
  args = "<description>"
  run  = <<-SHELL
    wok new bug "${args.description}" -l ${const.prefix}
    oj worker start bug
  SHELL
}

# File a wok chore and dispatch it to a worker.
command "chore" {
  args = "<description>"
  run  = <<-SHELL
    wok new chore "${args.description}" -l ${const.prefix}
    oj worker start chore
  SHELL
}

queue "bugs" {
  type = "external"
  list = "wok ready -t bug -p ${const.prefix} -o json"
  take = "wok start ${item.id}"
  poll = "30s"
}

queue "chores" {
  type = "external"
  list = "wok ready -t chore -p ${const.prefix} -o json"
  take = "wok start ${item.id}"
  poll = "30s"
}

worker "bug" {
  source      = { queue = "bugs" }
  handler     = { job = "bug" }
  concurrency = 3
}

worker "chore" {
  source      = { queue = "chores" }
  handler     = { job = "chore" }
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
    base  = "main"
    title = "$(printf 'fix: %.75s' \"${var.bug.title}\")"
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

job "chore" {
  name      = "${var.task.title}"
  vars      = ["task"]
  on_cancel = { step = "cancel" }
  on_fail   = { step = "reopen" }

  workspace {
    git    = "worktree"
    branch = "chore/${var.task.id}-${workspace.nonce}"
  }

  locals {
    base  = "main"
    title = "$(printf 'chore: %.73s' \"${var.task.title}\")"
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

  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      if test "$(git rev-list --count HEAD ^origin/${local.base})" -gt 0; then
        git push origin "${workspace.branch}"
        wok done ${var.task.id}
        oj queue push merges --var branch="${workspace.branch}" --var title="${local.title}"
      elif wok show ${var.task.id} -o json | grep -q '"status":"done"'; then
        echo "Issue already resolved, no changes needed"
      else
        echo "No changes to submit" >&2
        exit 1
      fi
    SHELL
  }

  step "reopen" {
    run = "wok reopen ${var.task.id} --reason 'Chore job failed'"
  }

  step "cancel" {
    run = "wok close ${var.task.id} --reason 'Chore job cancelled'"
  }
}

agent "bugs" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = { action = "nudge", message = "Keep working. Fix the bug, write tests, run ${raw(const.check)}, and commit." }
  on_dead  = { action = "gate", run = "${raw(const.check)}" }

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
    5. Run `${raw(const.check)}` to verify
    6. Commit your changes
    7. Mark the issue as done: `wok done ${var.bug.id}`

    If the bug is already fixed (e.g. by a prior commit), skip to step 7.
  PROMPT
}

agent "chores" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = { action = "nudge", message = "Keep working. Complete the task, write tests, run ${raw(const.check)}, and commit." }
  on_dead  = { action = "gate", run = "${raw(const.check)}" }

  session "tmux" {
    color = "blue"
    title = "Chore: ${var.task.id}"
    status {
      left  = "${var.task.id}: ${var.task.title}"
      right = "${workspace.branch}"
    }
  }

  prime = ["wok show ${var.task.id}"]

  prompt = <<-PROMPT
    Complete the following task: ${var.task.id} - ${var.task.title}

    ## Steps

    1. Understand the task
    2. Find the relevant code
    3. Implement the changes
    4. Write or update tests
    5. Run `${raw(const.check)}` to verify
    6. Commit your changes
    7. Mark the issue as done: `wok done ${var.task.id}`

    If the task is already completed (e.g. by a prior commit), skip to step 7.
  PROMPT
}
