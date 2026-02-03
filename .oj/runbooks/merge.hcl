# Queue a branch for the local merge queue.
#
# Merges into main with conflict resolution, testing, and push.
#
# Examples:
#   oj run merge fix/foo-abc123 "fix: button color"
#   oj run merge feature/auth-def456 "feat: add authentication" --base develop

command "merge" {
  args = "<branch> <title> [--base <base>]"
  run  = <<-SHELL
    oj queue push merges --var branch="${args.branch}" --var title="${args.title}" --var base="${args.base}"
    echo "Queued '${args.branch}' for merge"
  SHELL

  defaults = {
    base = "main"
  }
}

queue "merges" {
  type     = "persisted"
  vars     = ["branch", "title", "base"]
  defaults = { base = "main" }
}

worker "merge" {
  source      = { queue = "merges" }
  handler     = { pipeline = "merge" }
  concurrency = 1
}

pipeline "merge" {
  name      = "${var.mr.title}"
  vars      = ["mr"]
  workspace = "ephemeral"

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "merge-${workspace.nonce}"
  }

  notify {
    on_start = "Merging: ${var.mr.title}"
    on_done  = "Merged: ${var.mr.title}"
    on_fail  = "Merge failed: ${var.mr.title}"
  }

  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" fetch origin ${var.mr.base} ${var.mr.branch}
      git -C "${local.repo}" worktree add -b ${local.branch} "${workspace.root}" origin/${var.mr.base}
    SHELL
    on_done = { step = "merge" }
  }

  step "merge" {
    run     = "git merge origin/${var.mr.branch} --no-edit"
    on_done = { step = "check" }
    on_fail = { step = "resolve" }
  }

  step "check" {
    run     = "make check"
    on_done = { step = "push" }
    on_fail = { step = "resolve" }
  }

  step "resolve" {
    run     = { agent = "resolver" }
    on_done = { step = "push" }
  }

  step "push" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit --amend --no-edit
      git rebase --abort 2>/dev/null || true
      git -C "${local.repo}" fetch origin ${var.mr.base}
      git rebase origin/${var.mr.base}
      git -C "${local.repo}" push origin ${local.branch}:${var.mr.base}
      git -C "${local.repo}" push origin --delete ${var.mr.branch} || true
    SHELL
    on_done = { step = "cleanup" }
    on_fail = { step = "check", attempts = 2 }
  }

  step "cleanup" {
    run = <<-SHELL
      git -C "${local.repo}" worktree remove --force "${workspace.root}" 2>/dev/null || true
      git -C "${local.repo}" branch -D "${local.branch}" 2>/dev/null || true
    SHELL
  }
}

agent "resolver" {
  run      = "claude --model opus --dangerously-skip-permissions"
  on_idle  = { action = "gate", run = "make check", attempts = 2 }
  on_dead  = { action = "escalate" }

  prime = [
    "echo '## Git Status'",
    "git status",
    "echo '## Incoming Commits'",
    "git log origin/${var.mr.base}..origin/${var.mr.branch}",
    "echo '## Changed Files'",
    "git diff --stat origin/${var.mr.base}..origin/${var.mr.branch}",
  ]

  prompt = <<-PROMPT
    You are merging branch ${var.mr.branch} into ${var.mr.base}.

    Title: ${var.mr.title}

    The previous step failed -- either a merge conflict or a test failure.

    1. Run `git status` to check for merge conflicts
    2. If conflicts exist, resolve them and `git add` the files
    3. If mid-merge, run `git commit --no-edit` to complete it
    4. Run `make check` to verify everything passes
    5. Fix any test failures
    6. When `make check` passes, say "I'm done"
  PROMPT
}
