# GitHub PR merge queue with conflict resolution.
#
# Clean rebases flow through the fast-path. Conflicts and build failures
# are forwarded to a resolve queue where an agent handles resolution.
#
# Prerequisites:
#   - GitHub CLI (gh) installed and authenticated
#   - Repository must have auto-merge enabled in settings
#
# Consts:
#   check - verification command (default: "true")

const "check" { default = "true" }

# ------------------------------------------------------------------------------
# Merge queue (fast-path: clean rebases only)
# ------------------------------------------------------------------------------

queue "merges" {
  type = "external"
  list = "gh pr list --label merge:auto --json number,title,headRefName"
  take = "gh pr edit ${item.number} --add-label in-progress"
  poll = "30s"
}

worker "merge" {
  source      = { queue = "merges" }
  handler     = { job = "merge" }
  concurrency = 1
}

job "merge" {
  name      = "Merge PR #${var.pr.number}: ${var.pr.title}"
  vars      = ["pr"]
  workspace = "folder"
  on_cancel = { step = "cleanup" }

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "merge-pr-${var.pr.number}-${workspace.nonce}"
  }

  notify {
    on_start = "Merging PR #${var.pr.number}: ${var.pr.title}"
    on_done  = "Merged PR #${var.pr.number}: ${var.pr.title}"
    on_fail  = "Merge failed PR #${var.pr.number}: ${var.pr.title}"
  }

  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree remove --force "${workspace.root}" 2>/dev/null || true
      git -C "${local.repo}" branch -D "${local.branch}" 2>/dev/null || true
      git -C "${local.repo}" fetch origin main
      git -C "${local.repo}" fetch origin pull/${var.pr.number}/head:${local.branch}
      git -C "${local.repo}" worktree add "${workspace.root}" ${local.branch}
    SHELL
    on_done = { step = "rebase" }
  }

  step "rebase" {
    run     = "git rebase origin/main"
    on_done = { step = "verify" }
    on_fail = { step = "queue-cicd" }
  }

  step "verify" {
    run     = "${raw(const.check)}"
    on_done = { step = "push" }
    on_fail = { step = "queue-cicd" }
  }

  step "queue-cicd" {
    run = <<-SHELL
      git rebase --abort 2>/dev/null || true
      gh pr edit ${var.pr.number} --remove-label merge:auto --add-label merge:cicd
      oj worker start cicd
    SHELL
    on_done = { step = "cleanup" }
  }

  step "push" {
    run = <<-SHELL
      git push --force-with-lease origin HEAD:${var.pr.headRefName}
      gh pr merge ${var.pr.number} --squash --auto
      gh pr edit ${var.pr.number} --remove-label in-progress
      issue=$(gh pr view ${var.pr.number} --json body -q '.body' | grep -oE 'Closes #[0-9]+' | grep -oE '[0-9]+' | head -1)
      if [ -n "$issue" ]; then
        gh issue edit "$issue" --remove-label build:ready
      fi
    SHELL
    on_done = { step = "cleanup" }
  }

  step "cleanup" {
    run = <<-SHELL
      git -C "${local.repo}" worktree remove --force "${workspace.root}" 2>/dev/null || true
      git -C "${local.repo}" branch -D "${local.branch}" 2>/dev/null || true
    SHELL
  }
}

# ------------------------------------------------------------------------------
# CI/CD resolve queue (slow-path: agent-assisted resolution)
# ------------------------------------------------------------------------------

queue "cicd" {
  type = "external"
  list = "gh pr list --label merge:cicd --json number,title,headRefName"
  take = "gh pr edit ${item.number} --add-label in-progress"
  poll = "30s"
}

worker "cicd" {
  source      = { queue = "cicd" }
  handler     = { job = "cicd" }
  concurrency = 1
}

job "cicd" {
  name      = "Resolve PR #${var.pr.number}: ${var.pr.title}"
  vars      = ["pr"]
  workspace = "folder"
  on_cancel = { step = "cleanup" }

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "merge-pr-${var.pr.number}-${workspace.nonce}"
  }

  notify {
    on_start = "Resolving PR #${var.pr.number}: ${var.pr.title}"
    on_done  = "Resolved PR #${var.pr.number}: ${var.pr.title}"
    on_fail  = "Resolve failed PR #${var.pr.number}: ${var.pr.title}"
  }

  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree remove --force "${workspace.root}" 2>/dev/null || true
      git -C "${local.repo}" branch -D "${local.branch}" 2>/dev/null || true
      git -C "${local.repo}" fetch origin main
      git -C "${local.repo}" fetch origin pull/${var.pr.number}/head:${local.branch}
      git -C "${local.repo}" worktree add "${workspace.root}" ${local.branch}
    SHELL
    on_done = { step = "rebase" }
  }

  step "rebase" {
    run     = "git rebase origin/main"
    on_done = { step = "verify" }
    on_fail = { step = "resolve" }
  }

  step "verify" {
    run     = "${raw(const.check)}"
    on_done = { step = "push" }
    on_fail = { step = "resolve" }
  }

  step "resolve" {
    run     = { agent = "merge-resolver" }
    on_done = { step = "push" }
  }

  step "push" {
    run = <<-SHELL
      git push --force-with-lease origin HEAD:${var.pr.headRefName}
      gh pr merge ${var.pr.number} --squash --auto
      gh pr edit ${var.pr.number} --remove-label merge:cicd,in-progress
      issue=$(gh pr view ${var.pr.number} --json body -q '.body' | grep -oE 'Closes #[0-9]+' | grep -oE '[0-9]+' | head -1)
      if [ -n "$issue" ]; then
        gh issue edit "$issue" --remove-label build:ready
      fi
    SHELL
    on_done = { step = "cleanup" }
  }

  step "cleanup" {
    run = <<-SHELL
      git -C "${local.repo}" worktree remove --force "${workspace.root}" 2>/dev/null || true
      git -C "${local.repo}" branch -D "${local.branch}" 2>/dev/null || true
    SHELL
  }
}

# ------------------------------------------------------------------------------
# Agent
# ------------------------------------------------------------------------------

agent "merge-resolver" {
  run     = "claude --model sonnet --dangerously-skip-permissions"
  on_idle = { action = "gate", command = "test ! -d $(git rev-parse --git-dir)/rebase-merge" }
  on_dead = { action = "escalate" }

  session "tmux" {
    color = "yellow"
    title = "Merge: PR #${var.pr.number}"
    status {
      left  = "${var.pr.title}"
      right = "${var.pr.headRefName} -> main"
    }
  }

  prime = [
    "echo '## Git Status'",
    "git status",
    "echo '## PR'",
    "gh pr view ${var.pr.number}",
    "echo '## Commits (branch vs main)'",
    "git log --oneline origin/main..HEAD 2>/dev/null || git log --oneline REBASE_HEAD~1..REBASE_HEAD 2>/dev/null || true",
    <<-PRIME
%{ if const.check != "true" }
    echo '## Recent build output'
    ${raw(const.check)} 2>&1 | tail -80 || true
%{ endif }
    PRIME
  ]

  prompt = <<-PROMPT
    You are landing PR #${var.pr.number} ("${var.pr.title}") onto main.

    Something went wrong — either a rebase conflict or a build failure.
    Diagnose from the git status and build output above, then fix it.

    If mid-rebase: resolve conflicts, `git add`, `git rebase --continue`, repeat.
    If build fails: fix the code, amend the commit.
%{ if const.check != "true" }

    Done when: rebase is complete and `${raw(const.check)}` passes.
%{ else }

    Done when: rebase is complete with no conflicts.
%{ endif }
  PROMPT
}
