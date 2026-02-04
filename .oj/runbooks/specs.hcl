# Periodic spec health check.
#
# Runs `cargo test -p oj-specs` every 20 minutes. If tests fail,
# an agent fixes them and submits to the merge queue.

cron "specs" {
  interval = "20m"
  run      = { pipeline = "specs" }
}

pipeline "specs" {
  name      = "specs-${workspace.nonce}"

  workspace {
    git = "worktree"
  }

  locals {
    branch = "fix/specs-${workspace.nonce}"
  }

  notify {
    on_done = "Specs fixed: ${workspace.branch}"
    on_fail = "Specs fix failed"
  }

  step "run" {
    run     = "cargo test -p oj-specs"
    on_fail = { step = "fix" }
  }

  step "fix" {
    run     = { agent = "specs" }
    on_done = { step = "submit" }
  }

  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "fix: repair failing specs"
      test "$(git rev-list --count HEAD ^origin/main)" -gt 0 || { echo "No changes to submit" >&2; exit 1; }
      git push origin "${workspace.branch}"
      oj queue push merges --var branch="${workspace.branch}" --var title="fix: repair failing specs"
    SHELL
  }
}

agent "specs" {
  run      = "claude --model sonnet --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = "done"
  on_dead  = { action = "gate", run = "cargo test -p oj-specs" }

  prompt = <<-PROMPT
    `cargo test -p oj-specs` is failing. Fix the failing tests.

    1. Run `cargo test -p oj-specs` to see which tests fail
    2. Read the failing test code and the code it tests
    3. Fix the issue -- prefer fixing code over fixing tests
    4. Run `cargo test -p oj-specs` to verify
    5. Run `make check` to ensure nothing else broke
    6. Commit your changes
  PROMPT
}
