# Wok-based issue queues for bugs, chores, and epics.
#
# Consts:
#   prefix - wok issue prefix (required)
#   check  - verification command (default: "true")
#   submit - post-push command (default: none)

const "prefix" {}
const "check"  { default = "true" }
const "submit" { default = "" }

# File a wok chore and dispatch it to a worker.
#
# Examples:
#   oj run chore "Update dependencies to latest versions"
#   oj run chore "Add missing test coverage for auth module"

command "chore" {
  args = "<description>"
  run  = <<-SHELL
    wok new chore "${args.description}" -p ${const.prefix}
    oj worker start chore
  SHELL
}

queue "chores" {
  type = "external"
  list = "wok ready -t chore -p ${const.prefix} -o json"
  take = "wok start ${item.id}"
  poll = "30s"
}

worker "chore" {
  source      = { queue = "chores" }
  handler     = { job = "chore" }
  concurrency = 3
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
        branch="${workspace.branch}" title="${local.title}"
        git push origin "$branch"
        wok done ${var.task.id}
        %{ if const.submit != "" }
        ${raw(const.submit)}
        %{ endif }
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

agent "chores" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_dead = { action = "resume", attempts = 1 }

  on_idle {
    action  = "nudge"
    message = <<-MSG
%{ if const.check != "true" }
      Keep working. Complete the task, write tests, verify with:
      ```
      ${raw(const.check)}
      ```
      Then commit your changes.
%{ else }
      Keep working. Complete the task, write tests, then commit your changes.
%{ endif }
    MSG
  }

  prime = [
    "wok show ${var.task.id}",
    <<-PRIME
    echo '## Workflow'
    echo
    echo '1. Understand the task and find the relevant code'
    echo '2. Implement the changes and write or update tests'
%{ if const.check != "true" ~}
    echo '3. Verify: `${raw(const.check)}` — changes REJECTED if this fails'
    echo '4. Commit your changes'
    echo '5. Mark done: `wok done ${var.task.id}`'
    echo
    echo 'If already completed by a prior commit, skip to step 5.'
%{ else ~}
    echo '3. Commit your changes'
    echo '4. Mark done: `wok done ${var.task.id}`'
    echo
    echo 'If already completed by a prior commit, skip to step 4.'
%{ endif ~}
    PRIME
  ]

  prompt = "Complete the following task: ${var.task.id} - ${var.task.title}"
}
