# GitHub Issues as a bug queue with PR-based merge.
#
# Consts:
#   check - verification command (default: "true")

const "check" { default = "true" }

# File a GitHub bug and dispatch it to a fix worker.
#
# Examples:
#   oj run fix "Button doesn't respond to clicks"
#   oj run fix "Login page crashes on empty password" "Repro steps..."
#   oj run fix "Fix after auth lands" --blocked 42
command "fix" {
  args = "<title> [body] [--blocked <numbers>]"
  run  = <<-SHELL
    labels="type:bug"
    body="${args.body}"
    _blocked="${args.blocked}"
    ${raw(const.blocked)}
    if [ -n "$body" ]; then
      url=$(gh issue create --label "$labels" --title "${args.title}" --body "$body")
    else
      url=$(gh issue create --label "$labels" --title "${args.title}")
    fi
    issue=$(basename "$url")
    gh issue lock "$issue" 2>/dev/null || true
    oj worker start bug
  SHELL

  defaults = {
    body    = ""
    blocked = ""
  }
}

queue "bugs" {
  type = "external"
  list = "gh issue list --label type:bug --state open --json number,title --search '-label:blocked -label:in-progress'"
  take = "gh issue edit ${item.number} --add-label in-progress; gh issue lock ${item.number} 2>/dev/null || true"
  poll = "30s"
}

worker "bug" {
  source      = { queue = "bugs" }
  run = { job = "bug" }
  concurrency = 3
}

job "bug" {
  name      = "${var.bug.title}"
  vars      = ["bug"]
  on_fail   = { step = "reopen" }
  on_cancel = { step = "cancel" }

  source {
    git    = true
    branch = "fix/${var.bug.number}-${source.nonce}"
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

  step "sync" {
    run     = "git fetch origin ${local.base} && git rebase origin/${local.base} || true"
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
      if test "$(git rev-list --count HEAD ^origin/${local.base})" -gt 0; then
        branch="${source.branch}"
        git push origin "$branch"
        gh pr create --title "${local.title}" --body "Closes #${var.bug.number}" --head "$branch" --label merge:auto
        gh pr merge --squash --delete-branch --auto || echo "auto-merge unavailable; merge cron will handle"
      elif gh issue view ${var.bug.number} --json state -q '.state' | grep -q 'CLOSED'; then
        echo "Issue already resolved, no changes needed"
      else
        echo "No changes to submit" >&2
        exit 1
      fi
    SHELL
  }

  step "reopen" {
    run = <<-SHELL
      gh issue edit ${var.bug.number} --remove-label in-progress
      gh issue reopen ${var.bug.number} 2>/dev/null || true
    SHELL
  }

  step "cancel" {
    run = "gh issue close ${var.bug.number}"
  }
}

agent "bugs" {
  run     = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
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

  prime = [
    "gh issue view ${var.bug.number}",
    <<-PRIME
    echo '## Workflow'
    echo
    echo '1. Understand the bug and find the relevant code'
    echo '2. Implement a fix and write or update tests'
%{ if const.check != "true" ~}
    echo '3. Verify: `${raw(const.check)}` — changes REJECTED if this fails'
    echo '4. Commit your changes'
    echo
    echo 'If already fixed by a prior commit, just commit a no-op.'
%{ else ~}
    echo '3. Commit your changes'
    echo
    echo 'If already fixed by a prior commit, just commit a no-op.'
%{ endif ~}
    PRIME
  ]

  prompt = "Fix GitHub issue #${var.bug.number}: ${var.bug.title}"
}
