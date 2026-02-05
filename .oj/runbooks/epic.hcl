# Plan and implement a large feature using two-phase worker queues.
#
# Creates an epic issue, then workers handle planning and implementation:
# 1. Plan worker explores codebase and writes plan to issue notes
# 2. Epic worker implements the plan and submits to merge queue
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

# Queue an existing issue for planning.
#
# Examples:
#   oj run plan oj-abc123
command "plan" {
  args = "<issue>"
  run  = <<-SHELL
    wok label ${args.issue} plan:needed
    wok reopen ${args.issue}
    oj worker start plan
  SHELL
}

# Queue an existing issue for implementation (skip planning).
#
# Examples:
#   oj run build oj-abc123
command "build" {
  args = "<issue>"
  run  = <<-SHELL
    wok label ${args.issue} build:needed plan:ready
    wok reopen ${args.issue}
    oj worker start epic
  SHELL
}

queue "plans" {
  type = "external"
  list = "wok list -t epic -s todo --unassigned -l plan:needed -p oj -o json"
  take = "wok start ${item.id}"
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

  step "plan" {
    run     = { agent = "plan" }
    on_done = { step = "mark-ready" }
  }

  step "mark-ready" {
    run = <<-SHELL
      wok unlabel ${var.epic.id} plan:needed
      wok label ${var.epic.id} plan:ready
      wok reopen ${var.epic.id}
    SHELL
  }

  step "reopen" {
    run = "wok reopen ${var.epic.id} --reason 'Planning failed'"
  }

  step "cancel" {
    run = "wok close ${var.epic.id} --reason 'Planning cancelled'"
  }
}

# Implementation queue: picks up issues ready to build (build:needed AND plan:ready)
queue "epics" {
  type = "external"
  list = "wok list -t epic -s todo --unassigned -l build:needed -l plan:ready -p oj -o json"
  take = "wok start ${item.id}"
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
    run = "wok reopen ${var.epic.id} --reason 'Epic failed'"
  }

  step "cancel" {
    run = "wok close ${var.epic.id} --reason 'Epic cancelled'"
  }
}

# ------------------------------------------------------------------------------
# Agents
# ------------------------------------------------------------------------------

agent "plan" {
  run = <<-CMD
    claude --model opus \
      --dangerously-skip-permissions \
      --disallowed-tools EnterPlanMode,ExitPlanMode,Write,Edit,NotebookEdit,TodoWrite,TodoRead
  CMD

  on_dead = { action = "gate", run = "wok show ${var.epic.id} -o json | jq -e '.notes | length > 0'" }

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
