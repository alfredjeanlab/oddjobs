# Chore Runbook
#
# Worker pool: worker pulls tasks from wok → do → verify → push
#
# Usage:
#   oj run chore <description>       # File a task and start the worker
#   oj worker start chore            # Start worker

command "chore" {
  args = "<description>"
  run  = <<-SHELL
    wok new chore "${args.description}"
    oj worker start chore
  SHELL
}

queue "chores" {
  type = "external"
  list = "wok list -t chore -s todo --unassigned -o json"
  take = "wok start ${item.id}"
}

worker "chore" {
  source      = { queue = "chores" }
  handler     = { pipeline = "chore" }
  concurrency = 1
}

pipeline "chore" {
  name      = "${var.task.title}"
  vars      = ["task"]
  workspace = "ephemeral"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "chore/${var.task.id}-${workspace.nonce}"
    title  = "chore: ${var.task.title}"
  }

  notify {
    on_start = "Chore: ${var.task.title}"
    on_done  = "Chore done: ${var.task.title}"
    on_fail  = "Chore failed: ${var.task.title}"
  }

  # Initialize workspace: worktree with shared build cache via .cargo/config.toml
  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" HEAD
      mkdir -p .cargo
      echo "[build]" > .cargo/config.toml
      echo "target-dir = \"${local.repo}/target\"" >> .cargo/config.toml
    SHELL
    on_done = { step = "work" }
  }

  step "work" {
    run     = { agent = "choreworker" }
    on_done = { step = "submit" }
  }

  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      git -C "${local.repo}" push origin "${local.branch}"
      oj queue push merges --var branch="${local.branch}" --var title="${local.title}"
    SHELL
    on_done = { step = "done" }
  }

  step "done" {
    run = "cd ${invoke.dir} && wok done ${var.task.id}"
  }
}

agent "choreworker" {
  run      = "claude --model opus --dangerously-skip-permissions"
  on_idle  = { action = "nudge", message = "Keep working. Complete the task, write tests, run make check, and commit." }
  on_dead  = { action = "gate", run = "make check" }

  prompt = <<-PROMPT
    Complete the following task:

    ${var.task.title}

    ## Steps

    1. Understand the task
    2. Find the relevant code
    3. Implement the changes
    4. Write or update tests
    5. Run `make check` to verify
    6. Commit your changes
  PROMPT
}
