# Periodic spec health check.
#
# Runs `cargo test -p oj-specs` every 20 minutes. If tests fail,
# an agent fixes them and submits to the merge queue.

cron "specs" {
  interval = "20m"
  run      = { job = "specs" }
}

job "specs" {
  name      = "specs-${workspace.nonce}"

  workspace {
    git    = "worktree"
    branch = "fix/specs-${workspace.nonce}"
  }

  notify {
    on_done = "Specs fixed: ${workspace.branch}"
    on_fail = "Specs fix failed"
  }

  step "build" {
    run     = "cargo build --all"
    on_done = { step = "run" }
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

# Periodic flaky test reduction.
#
# Every 4 hours, runs the spec suite multiple times to surface
# intermittent failures, then an agent fixes the root causes.

cron "deflake" {
  interval = "4h"
  run      = { job = "deflake" }
}

job "deflake" {
  name      = "deflake-${workspace.nonce}"

  workspace {
    git    = "worktree"
    branch = "fix/deflake-${workspace.nonce}"
  }

  notify {
    on_done = "Flaky tests fixed: ${workspace.branch}"
    on_fail = "Deflake failed"
  }

  step "build" {
    run     = "cargo build --all"
    on_done = { step = "detect" }
  }

  step "detect" {
    run     = <<-SHELL
      failures=""
      for i in 1 2 3 4 5; do
        if ! cargo test -p oj-specs 2>&1 | tee "/tmp/deflake-run-$i.log"; then
          failures="$failures $i"
        fi
      done
      test -n "$failures" || { echo "No flaky tests detected" >&2; exit 1; }
    SHELL
    on_fail = { step = "fix" }
  }

  step "fix" {
    run     = { agent = "deflake" }
    on_done = { step = "submit" }
  }

  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "fix: reduce test flakiness"
      test "$(git rev-list --count HEAD ^origin/main)" -gt 0 || { echo "No changes to submit" >&2; exit 1; }
      git push origin "${workspace.branch}"
      oj queue push merges --var branch="${workspace.branch}" --var title="fix: reduce test flakiness"
    SHELL
  }
}

agent "deflake" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
  on_idle  = "done"
  on_dead  = { action = "gate", run = "cargo test -p oj-specs" }

  prompt = <<-PROMPT
    The spec suite (`cargo test -p oj-specs`) has flaky tests — tests that
    sometimes pass and sometimes fail across repeated runs. The logs from
    5 consecutive runs are in /tmp/deflake-run-{1..5}.log.

    Your goal is to eliminate the flakiness, not just make tests pass once.

    1. Read /tmp/deflake-run-{1..5}.log to identify which tests are intermittent
    2. Read the flaky test code and the production code it exercises
    3. Identify the root cause (race conditions, timing dependencies, shared
       state, non-deterministic ordering, etc.)
    4. Fix the root cause — prefer fixing production code over test code, but
       fix whichever is actually wrong
    5. Run `cargo test -p oj-specs` several times to verify stability
    6. Run `make check` to ensure nothing else broke
    7. Commit your changes
  PROMPT
}

agent "specs" {
  run      = "claude --model opus --dangerously-skip-permissions --disallowed-tools ExitPlanMode,EnterPlanMode"
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
