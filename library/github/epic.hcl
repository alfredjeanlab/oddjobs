# GitHub Issues as an epic queue with planning and implementation.
#
# Workflow: create epic issue → plan worker explores and writes plan →
# build worker implements the plan → PR with auto-merge.
#
# Blocking: single `blocked` label. Dependencies tracked in issue body
# as "Blocked by: #2, #5, #14". An unblock cron detects when deps close
# and removes the blocked label.
#
# Consts:
#   check - verification command (default: "true")

const "check" { default = "true" }

# Create a new epic with 'plan:needed' and 'build:needed'.
#
# Examples:
#   oj run epic "Implement user authentication with OAuth"
#   oj run epic "Implement OAuth" "Support Google and GitHub providers"
#   oj run epic "Refactor storage layer" --after 5
#   oj run epic "Wire everything together" --after "3 5 14"
command "epic" {
  args = "<title> [body] [--after <numbers>]"
  run  = <<-SHELL
    labels="type:epic,plan:needed,build:needed"
    body="${args.body}"
    if [ -n "${args.after}" ]; then
      labels="$labels,blocked"
      refs=""
      for n in ${args.after}; do refs="$refs #$n"; done
      if [ -n "$body" ]; then
        body="$body\n\nBlocked by:$refs"
      else
        body="Blocked by:$refs"
      fi
    fi
    if [ -n "$body" ]; then
      gh issue create --label "$labels" --title "${args.title}" --body "$body"
    else
      gh issue create --label "$labels" --title "${args.title}"
    fi
    oj worker start plan
    oj worker start epic
  SHELL

  defaults = {
    body  = ""
    after = ""
  }
}

# Create a new epic with 'plan:needed' only (no auto-build).
#
# Examples:
#   oj run idea "Add caching layer for API responses"
#   oj run idea "Prototype new UI layout" "Explore grid vs flex"
command "idea" {
  args = "<title> [body]"
  run  = <<-SHELL
    if [ -n "${args.body}" ]; then
      gh issue create --label type:epic,plan:needed --title "${args.title}" --body "${args.body}"
    else
      gh issue create --label type:epic,plan:needed --title "${args.title}"
    fi
    oj worker start plan
  SHELL

  defaults = {
    body = ""
  }
}

# Queue existing issues for planning.
#
# Examples:
#   oj run plan 42
#   oj run plan 42 43
command "plan" {
  args = "<issues>"
  run  = <<-SHELL
    for num in ${args.issues}; do
      gh issue edit "$num" --add-label plan:needed
      gh issue reopen "$num" 2>/dev/null || true
    done
    oj worker start plan
  SHELL
}

# Queue existing issues for building (requires plan:ready).
#
# Examples:
#   oj run build 42
#   oj run build 42 43
command "build" {
  args = "<issues>"
  run  = <<-SHELL
    for num in ${args.issues}; do
      if ! gh issue view "$num" --json labels -q '.labels[].name' | grep -q '^plan:ready$'; then
        echo "error: #$num is missing 'plan:ready' label" >&2
        exit 1
      fi
    done
    for num in ${args.issues}; do
      gh issue edit "$num" --add-label build:needed
      gh issue reopen "$num" 2>/dev/null || true
    done
    oj worker start epic
  SHELL
}

# Check all blocked issues and remove label when all deps are resolved.
#
# Examples:
#   oj run unblock
command "unblock" {
  run = <<-SHELL
    gh issue list --label blocked --state open --json number,body | jq -c '.[]' | while read -r obj; do
      num=$(echo "$obj" | jq -r .number)
      deps=$(echo "$obj" | jq -r '.body' | grep -i 'Blocked by:' | grep -oE '#[0-9]+' | grep -oE '[0-9]+')
      if [ -z "$deps" ]; then
        gh issue edit "$num" --remove-label blocked
        echo "Unblocked #$num (no deps)"
        continue
      fi
      all_closed=true
      for dep in $deps; do
        state=$(gh issue view "$dep" --json state -q .state 2>/dev/null)
        if [ "$state" != "CLOSED" ]; then
          all_closed=false
          break
        fi
      done
      if [ "$all_closed" = true ]; then
        gh issue edit "$num" --remove-label blocked
        echo "Unblocked #$num"
      fi
    done
  SHELL
}

# ------------------------------------------------------------------------------
# Plan queue and worker
# ------------------------------------------------------------------------------

queue "plans" {
  type = "external"
  list = "gh issue list --label type:epic,plan:needed --state open --json number,title --search '-label:blocked -label:in-progress'"
  take = "gh issue edit ${item.number} --add-label in-progress"
  poll = "30s"
}

worker "plan" {
  source      = { queue = "plans" }
  handler     = { job = "plan" }
  concurrency = 5
}

job "plan" {
  name      = "Plan: ${var.epic.title}"
  vars      = ["epic"]
  on_fail   = { step = "reopen" }
  on_cancel = { step = "cancel" }

  workspace {
    git    = "worktree"
    branch = "plan/${var.epic.number}-${workspace.nonce}"
  }

  locals {
    base = "main"
  }

  step "sync" {
    run     = "git fetch origin ${local.base} && git rebase origin/${local.base} || true"
    on_done = { step = "think" }
  }

  step "think" {
    run     = { agent = "plan" }
    on_done = { step = "planned" }
  }

  step "planned" {
    run = <<-SHELL
      gh issue edit ${var.epic.number} --remove-label plan:needed,in-progress --add-label plan:ready
      gh issue reopen ${var.epic.number} 2>/dev/null || true
      oj worker start epic
    SHELL
  }

  step "reopen" {
    run = <<-SHELL
      gh issue edit ${var.epic.number} --remove-label plan:needed,in-progress --add-label plan:failed
      gh issue reopen ${var.epic.number} 2>/dev/null || true
    SHELL
  }

  step "cancel" {
    run = "gh issue close ${var.epic.number}"
  }
}

# ------------------------------------------------------------------------------
# Epic (build) queue and worker
# ------------------------------------------------------------------------------

queue "epics" {
  type = "external"
  list = "gh issue list --label type:epic,plan:ready,build:needed --state open --json number,title --search '-label:blocked -label:in-progress'"
  take = "gh issue edit ${item.number} --add-label in-progress"
  poll = "30s"
}

worker "epic" {
  source      = { queue = "epics" }
  handler     = { job = "epic" }
  concurrency = 5
}

job "epic" {
  name      = "${var.epic.title}"
  vars      = ["epic"]
  on_fail   = { step = "reopen" }
  on_cancel = { step = "cancel" }

  workspace {
    git    = "worktree"
    branch = "epic/${var.epic.number}-${workspace.nonce}"
  }

  locals {
    base  = "main"
    title = "$(printf 'feat: %.76s' \"${var.epic.title}\")"
  }

  notify {
    on_start = "Building: ${var.epic.title}"
    on_done  = "Built: ${var.epic.title}"
    on_fail  = "Build failed: ${var.epic.title}"
  }

  step "sync" {
    run     = "git fetch origin ${local.base} && git rebase origin/${local.base} || true"
    on_done = { step = "implement" }
  }

  step "implement" {
    run     = { agent = "implement" }
    on_done = { step = "submit" }
  }

  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      if test "$(git rev-list --count HEAD ^origin/${local.base})" -gt 0; then
        branch="${workspace.branch}"
        git push origin "$branch"
        gh pr create --title "${local.title}" --body "Closes #${var.epic.number}" --head "$branch" --label merge:auto
        gh issue edit ${var.epic.number} --remove-label build:needed,in-progress --add-label build:ready
        oj worker start merge
      else
        echo "No changes" >&2
        exit 1
      fi
    SHELL
  }

  step "reopen" {
    run = <<-SHELL
      gh issue edit ${var.epic.number} --remove-label build:needed,in-progress --add-label build:failed
      gh issue reopen ${var.epic.number} 2>/dev/null || true
    SHELL
  }

  step "cancel" {
    run = "gh issue close ${var.epic.number}"
  }
}

# ------------------------------------------------------------------------------
# Unblock cron
# ------------------------------------------------------------------------------

cron "unblock" {
  interval = "60s"
  run      = { job = "unblock" }
}

job "unblock" {
  name = "unblock"

  step "check" {
    run = <<-SHELL
      gh issue list --label blocked --state open --json number,body | jq -c '.[]' | while read -r obj; do
        num=$(echo "$obj" | jq -r .number)
        deps=$(echo "$obj" | jq -r '.body' | grep -i 'Blocked by:' | grep -oE '#[0-9]+' | grep -oE '[0-9]+')
        if [ -z "$deps" ]; then
          gh issue edit "$num" --remove-label blocked
          echo "Unblocked #$num (no deps)"
          continue
        fi
        all_closed=true
        for dep in $deps; do
          state=$(gh issue view "$dep" --json state -q .state 2>/dev/null)
          if [ "$state" != "CLOSED" ]; then
            all_closed=false
            break
          fi
        done
        if [ "$all_closed" = true ]; then
          gh issue edit "$num" --remove-label blocked
          echo "Unblocked #$num"
        fi
      done
    SHELL
  }
}

# ------------------------------------------------------------------------------
# Agents
# ------------------------------------------------------------------------------

agent "plan" {
  run     = "claude --model opus --dangerously-skip-permissions --disallowed-tools EnterPlanMode,ExitPlanMode"
  on_dead = { action = "resume", attempts = 1 }

  session "tmux" {
    color = "blue"
    title = "Plan: #${var.epic.number}"
    status { left = "#${var.epic.number}: ${var.epic.title}" }
  }

  prime = [
    "gh issue view ${var.epic.number}",
    <<-PRIME
    echo '## Workflow'
    echo
    echo '1. Spawn 3-5 Explore agents in parallel to understand the codebase'
    echo '2. Spawn a Plan agent to synthesize findings into a plan'
    echo '3. Add the plan as a comment: `gh issue comment ${var.epic.number} -b "the plan"`'
    echo
    echo 'The job will not advance until a comment is added to the issue.'
    PRIME
  ]

  prompt = "Create an implementation plan for GitHub issue #${var.epic.number}: ${var.epic.title}"
}

agent "implement" {
  run     = "claude --model opus --dangerously-skip-permissions --disallowed-tools EnterPlanMode,ExitPlanMode"
  on_dead = { action = "resume", attempts = 1 }

  on_idle {
    action  = "nudge"
    message = <<-MSG
%{ if const.check != "true" }
      Follow the plan, implement, test, then verify with:
      ```
      ${raw(const.check)}
      ```
      Then commit your changes.
%{ else }
      Follow the plan, implement, test, then commit your changes.
%{ endif }
    MSG
  }

  session "tmux" {
    color = "blue"
    title = "Epic: #${var.epic.number}"
    status {
      left  = "#${var.epic.number}: ${var.epic.title}"
      right = "${workspace.branch}"
    }
  }

  prime = [
    "gh issue view ${var.epic.number} --comments",
    <<-PRIME
    echo '## Workflow'
    echo
    echo 'The plan is in the issue comments above.'
    echo
    echo '1. Follow the plan and implement the changes'
    echo '2. Write or update tests'
%{ if const.check != "true" ~}
    echo '3. Verify: `${raw(const.check)}` — changes REJECTED if this fails'
    echo '4. Commit your changes'
%{ else ~}
    echo '3. Commit your changes'
%{ endif ~}
    PRIME
  ]

  prompt = "Implement GitHub issue #${var.epic.number}: ${var.epic.title}"
}
