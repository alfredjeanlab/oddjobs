# Plan and implement a large 'epic' wok issues with the 'plan:needed' and 'build:needed' labels.
#
# Creates an epic issue, then workers handle planning and implementation:
# 1. Plan worker explores codebase and writes plan to issue notes
# 2. Epic worker implements the plan and submits to merge queue

# Create a new wok epic with 'plan:needed' and 'build:needed'.
#
# Examples:
#   oj run epic "Implement user authentication with OAuth"
#   oj run epic "Refactor storage layer for multi-tenancy"
command "epic" {
  args = "<description>"
  run  = <<-SHELL
    wok new epic "${args.description}" -l plan:needed -l build:needed
    oj worker start plan
    oj worker start epic
  SHELL
}

# Create a new wok epic with 'plan:needed' only.
#
# Examples:
#   oj run plan oj-abc123
#   oj run plan oj-abc123 oj-def456
command "idea" {
  args = "<description>"
  run  = <<-SHELL
    wok new epic "${args.description}" -l plan:needed
    oj worker start plan
  SHELL
}

# Queue existing feature/epic for planning, adding the 'plan:needed' label.
#
# Examples:
#   oj run plan oj-abc123
#   oj run plan oj-abc123 oj-def456
command "plan" {
  args = "<issues>"
  run  = <<-SHELL
    wok label ${args.issues} plan:needed
    wok reopen ${args.issues}
    oj worker start plan
  SHELL
}

# Queue existing feature/epic for planning, adding the 'build:needed' label.
#
# Examples:
#   oj run build oj-abc123
#   oj run build oj-abc123 oj-def456
command "build" {
  args = "<issues>"
  run  = <<-SHELL
    wok label ${args.issues} build:needed
    wok reopen ${args.issues}
    oj worker start epic
  SHELL
}

queue "plans" {
  type = "external"
  list = "wok ready -t epic,feature -l plan:needed -p oj -o json"
  take = "wok start ${item.id}"
  poll = "30s"
}

worker "plan" {
  source      = { queue = "plans" }
  handler     = { job = "plan" }
  concurrency = 3
}

job "plan" {
  name      = "Plan: ${var.epic.title}"
  vars      = ["epic"]
  on_fail   = { step = "reopen" }
  on_cancel = { step = "cancel" }

  step "think" {
    run     = { agent = "plan" }
    on_done = { step = "planned" }
  }

  step "planned" {
    run = <<-SHELL
      wok unlabel ${var.epic.id} plan:needed
      wok label ${var.epic.id} plan:ready
      wok reopen ${var.epic.id}
      oj worker start epic
    SHELL
  }

  step "reopen" {
    run = <<-SHELL
      wok unlabel ${var.epic.id} plan:needed
      wok label ${var.epic.id} plan:failed
      wok reopen ${var.epic.id} --reason 'Planning failed'
    SHELL
  }

  step "cancel" {
    run = "wok close ${var.epic.id} --reason 'Planning cancelled'"
  }
}

queue "epics" {
  type = "external"
  list = "wok ready -t epic,feature -l build:needed -l plan:ready -p oj -o json"
  take = "wok start ${item.id}"
  poll = "30s"
}

worker "epic" {
  source      = { queue = "epics" }
  handler     = { job = "epic" }
  concurrency = 2
}

job "epic" {
  name      = "${var.epic.title}"
  vars      = ["epic"]
  on_fail   = { step = "reopen" }
  on_cancel = { step = "cancel" }

  workspace {
    git    = "worktree"
    branch = "epic/${var.epic.id}-${workspace.nonce}"
  }

  locals {
    base  = "main"
    title = "$(printf 'feat: %.76s' \"${var.epic.title}\")"
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
        git push origin "${workspace.branch}"
        wok done ${var.epic.id}
        oj queue push merges --var branch="${workspace.branch}" --var title="${local.title}"
      else
        echo "No changes" >&2
        exit 1
      fi
    SHELL
  }

  step "reopen" {
    run = <<-SHELL
      wok unlabel ${var.epic.id} build:needed
      wok label ${var.epic.id} build:failed
      wok reopen ${var.epic.id} --reason 'Epic failed'
    SHELL
  }

  step "cancel" {
    run = "wok close ${var.epic.id} --reason 'Epic cancelled'"
  }
}

# ------------------------------------------------------------------------------
# Agents
# ------------------------------------------------------------------------------

agent "plan" {
  run = "claude --model opus --dangerously-skip-permissions --disallowed-tools EnterPlanMode,ExitPlanMode"
  on_dead = { action = "gate", run = "wok show ${var.epic.id} -o json | jq -e '.notes | length > 0'" }

  session "tmux" {
    color = "blue"
    title = "Plan: ${var.epic.id}"
    status { left = "${var.epic.id}: ${var.epic.title}" }
  }

  prime = ["wok show ${var.epic.id}"]

  prompt = <<-PROMPT
    Create an implementation plan for: ${var.epic.id} - ${var.epic.title}

    1. Spawn 3-5 Explore agents in parallel (depending on complexity)
    2. Spawn a Plan agent to synthesize findings
    3. Add the plan: `wok note ${var.epic.id} "the plan"`
  PROMPT
}

agent "implement" {
  run     = "claude --model opus --dangerously-skip-permissions --disallowed-tools EnterPlanMode,ExitPlanMode"
  on_idle = { action = "nudge", message = "Follow the plan, implement, test, commit." }
  on_dead = { action = "gate", run = "make check" }

  session "tmux" {
    color = "blue"
    title = "Epic: ${var.epic.id}"
    status {
      left  = "${var.epic.id}: ${var.epic.title}"
      right = "${workspace.branch}"
    }
  }

  prime = ["wok show ${var.epic.id} --notes"]

  prompt = <<-PROMPT
    Implement: ${var.epic.id} - ${var.epic.title}

    The plan is in the issue notes above.

    1. Follow the plan
    2. Implement
    3. Run `make check`
    4. Commit
    5. Run: `wok done ${var.epic.id}`
  PROMPT
}
