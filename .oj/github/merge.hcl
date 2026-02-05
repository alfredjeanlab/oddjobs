queue "github-prs" {
  type = "external"
  list = "gh pr list --json number,title,headRefName --label auto-merge"
  take = "echo ${item.number}"
}

worker "github-merge" {
  source  = { queue = "github-prs" }
  handler = { job = "merge" }
}

job "merge" {
  vars = ["pull"]

  step "rebase" {
    run = <<-SHELL
      gh pr checkout ${var.pull.number}
      git fetch origin main
      git rebase origin/main
      git push --force-with-lease
    SHELL
    on_done = { step = "check" }
    on_fail = { step = "resolve" }
  }

  step "check" {
    run     = "make check"
    on_done = { step = "merge" }
    on_fail = { step = "resolve" }
  }

  step "merge" {
    run = "gh pr merge ${var.pull.number} --squash"
  }

  step "resolve" {
    run     = { agent = "resolver" }
    on_done = { step = "check" }
  }

  step "notify" {
    run = "gh pr comment ${var.pull.number} --body 'Merge failed'"
  }
}

agent "resolver" {
  run      = "claude --dangerously-skip-permissions"
  on_idle  = { action = "gate", run = "make check", attempts = 5 }
  on_dead  = { action = "escalate" }

  prompt = <<-PROMPT
    You are resolving issues for PR #${var.pull.number}: ${var.pull.title}

    The previous step failed -- either a rebase conflict or a test failure.

    1. Run `git status` to check for conflicts
    2. If conflicts exist, resolve them and `git add` the files
    3. If mid-rebase, run `git rebase --continue`
    4. Run `make check` to verify everything passes
    5. Fix any test failures
    6. Force-push: `git push --force-with-lease`
    7. When `make check` passes, say "I'm done"
  PROMPT
}
