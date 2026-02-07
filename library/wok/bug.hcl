# Wok-based issue queues for bugs, chores, and epics.
#
# Consts:
#   prefix - wok issue prefix (required)
#   check  - verification command (default: "true")
#   submit - post-push command (default: none)

const "prefix" {}
const "check"  { default = "true" }
const "submit" { default = "" }

# File a wok bug and dispatch it to a fix worker.
#
# Examples:
#   oj run fix "Button doesn't respond to clicks"
#   oj run fix "Login page crashes on empty password"
command "fix" {
  args = "<description>"
  run  = <<-SHELL
    wok new bug "${args.description}" -p ${const.prefix}
    oj worker start bug
  SHELL
}

queue "bugs" {
  type = "external"
  list = "wok ready -t bug -p ${const.prefix} -o json"
  take = "wok start ${item.id}"
  poll = "30s"
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
        branch="${workspace.branch}" title="${local.title}"
        git push origin "$branch"
        wok done ${var.bug.id}
        %{ if const.submit != "" }
        ${raw(const.submit)}
        %{ endif }
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
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_dead = { action = "resume", attempts = 1 }

  on_idle {
    action  = "nudge"
    message = <<-MSG
%{ if const.check != "true" }
      Keep working. Fix the bug, write tests, verify with:
      ```
      ${raw(const.check)}
      ```
      Then commit your changes.
%{ else }
      Keep working. Fix the bug, write tests, then commit your changes.
%{ endif }
    MSG
  }

  session "tmux" {
    color = "blue"
    title = "Bug: ${var.bug.id}"
    status {
      left  = "${var.bug.id}: ${var.bug.title}"
      right = "${workspace.branch}"
    }
  }

  prime = [
    "wok show ${var.bug.id}",
    <<-PRIME
    echo '## Workflow'
    echo
    echo '1. Understand the bug and find the relevant code'
    echo '2. Implement a fix and write or update tests'
%{ if const.check != "true" ~}
    echo '3. Verify: `${raw(const.check)}` — changes REJECTED if this fails'
    echo '4. Commit your changes'
    echo '5. Mark done: `wok done ${var.bug.id}`'
    echo
    echo 'If already fixed by a prior commit, skip to step 5.'
%{ else ~}
    echo '3. Commit your changes'
    echo '4. Mark done: `wok done ${var.bug.id}`'
    echo
    echo 'If already fixed by a prior commit, skip to step 4.'
%{ endif ~}
    PRIME
  ]

  prompt = "Fix the following bug: ${var.bug.id} - ${var.bug.title}"
}
