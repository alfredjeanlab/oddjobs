# Local Merge Queue
# Merges branches into main locally, with conflict resolution and testing.
#
# Usage:
#   oj queue push merges '{"branch": "fix-123", "title": "fix: button color"}'
#   oj worker start merge

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
  name      = "${var.mr.branch}"
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
      mkdir -p .cargo
      echo "[build]" > .cargo/config.toml
      echo "target-dir = \"${local.repo}/target\"" >> .cargo/config.toml
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
      git -C "${local.repo}" push origin ${local.branch}:${var.mr.base}
      git -C "${local.repo}" push origin --delete ${var.mr.branch}
    SHELL
  }
}

agent "resolver" {
  run      = "claude --model opus --dangerously-skip-permissions"
  on_idle  = { action = "gate", run = "make check", attempts = 5 }
  on_dead  = { action = "escalate" }

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
