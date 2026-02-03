# Formula: Refinery Patrol — Sequential Merge Queue Processor
#
# Gas Town equivalent: mol-refinery-patrol.formula.toml
#
# The Refinery is Gas Town's merge queue processor. It receives completed
# polecat work, rebases branches sequentially on main, runs tests, and pushes.
#
# Key principles:
#   - Rebase-as-work: conflicts spawn fresh polecats, never "sent back"
#   - Sequential rebasing: one at a time prevents cascading conflicts
#   - Agent-driven decisions (ZFC #5): Claude decides on conflicts, not code
#   - Auto-merge, no approval gate: tests passing IS the gate
#   - Non-blocking delegation: conflicts don't stop the queue
#   - Strict post-merge: push → notify witness → close MR → cleanup

pipeline "refinery-patrol" {
  name      = "refinery-${var.mr.id}"
  vars      = ["mr"]
  workspace = "ephemeral"

  locals {
    repo = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
  }

  notify {
    on_start = "Refinery: ${var.mr.id}"
    on_done  = "Merged: ${var.mr.id}"
    on_fail  = "Merge failed: ${var.mr.id}"
  }

  # Fetch branch and create worktree from the MR branch
  step "init" {
    run = <<-SHELL
      MR_ID="${var.mr.id}"

      # Extract MR metadata from beads
      MR_JSON=$(bd show "$MR_ID" --json 2>/dev/null || echo '{}')
      BRANCH=$(echo "$MR_JSON" | jq -r '.labels[]' 2>/dev/null | grep '^branch:' | cut -d: -f2 || echo "")
      BASE=$(echo "$MR_JSON" | jq -r '.labels[]' 2>/dev/null | grep '^base:' | cut -d: -f2 || echo "main")
      ISSUE=$(echo "$MR_JSON" | jq -r '.labels[]' 2>/dev/null | grep '^issue:' | cut -d: -f2 || echo "")

      if [ -z "$BRANCH" ]; then
        echo "Error: could not determine branch from MR $MR_ID"
        exit 1
      fi

      # Fetch and create worktree from the MR branch
      git -C "${local.repo}" fetch origin "$BASE" "$BRANCH"
      git -C "${local.repo}" worktree add -b "refinery-${workspace.nonce}" "${workspace.root}" "origin/$BRANCH"

      echo "$BRANCH" > .mr-branch
      echo "$BASE" > .mr-base
      echo "$ISSUE" > .mr-issue
      echo "$MR_ID" > .mr-id
    SHELL
    on_done = { step = "rebase" }
  }

  # Rebase MR branch onto current base (sequential rebase protocol)
  step "rebase" {
    run     = "git rebase origin/$(cat .mr-base)"
    on_done = { step = "check" }
    on_fail = { step = "resolve" }
  }

  # Run verification
  step "check" {
    run     = "make check"
    on_done = { step = "push" }
    on_fail = { step = "resolve" }
  }

  # Agent-driven conflict/failure resolution (ZFC #5)
  step "resolve" {
    run     = { agent = "refinery-agent" }
    on_done = { step = "push" }
  }

  # Strict post-merge sequence: push → notify → close → cleanup
  step "push" {
    run = <<-SHELL
      MR_ID="$(cat .mr-id)"
      BRANCH="$(cat .mr-branch)"
      BASE="$(cat .mr-base)"
      ISSUE="$(cat .mr-issue)"

      # 1. Push to target branch
      MERGE_BRANCH="$(git branch --show-current)"
      git -C "${local.repo}" fetch origin "$BASE"
      git rebase "origin/$BASE"
      git -C "${local.repo}" push origin "$MERGE_BRANCH:$BASE"

      # 2. Send MERGED mail to witness (REQUIRED before cleanup)
      bd create -t message \
        --title "MERGED" \
        --description "Branch: $BRANCH\nIssue: $ISSUE\nMR: $MR_ID\nBase: $BASE\nMerged-At: $(date -u +%Y-%m-%dT%H:%M:%SZ)" \
        --labels "from:refinery,to:witness,msg-type:merged" 2>/dev/null || true

      # 3. Close MR bead (REQUIRED for audit trail)
      bd close "$MR_ID" --reason "Merged to $BASE" 2>/dev/null || true

      # 4. Cleanup: delete polecat branch
      git -C "${local.repo}" push origin --delete "$BRANCH" 2>/dev/null || true
    SHELL
    on_done = { step = "cleanup" }
    on_fail = { step = "check", attempts = 2 }
  }

  step "cleanup" {
    run = <<-SHELL
      git -C "${local.repo}" worktree remove --force "${workspace.root}" 2>/dev/null || true
      git -C "${local.repo}" branch -D "refinery-${workspace.nonce}" 2>/dev/null || true
    SHELL
  }
}

# Refinery agent — resolves conflicts and test failures
# Gas Town: "The agent makes all merge/conflict decisions, not Go code" (ZFC #5)
agent "refinery-agent" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "gate", run = "make check", attempts = 5 }
  on_dead  = { action = "escalate" }

  env = {
    BD_ACTOR        = "refinery"
    GIT_AUTHOR_NAME = "refinery"
    GT_ROLE         = "refinery"
    GT_SCOPE        = "rig"
  }

  prompt = <<-PROMPT
    You are the Refinery — the merge queue processor.

    A previous step failed (merge conflict or test failure).

    ## Your Decision (Agent-Driven — ZFC #5)

    You make the call:
    - **Trivial conflicts** (whitespace, imports): resolve directly
    - **Test failures from the branch**: fix them
    - **Pre-existing test failures**: file a bug bead and proceed
    - **Complex conflicts**: escalate

    ## Steps

    1. `git status` — check for merge conflicts
    2. If conflicts: examine, resolve if trivial, `git add`, `git rebase --continue`
    3. If tests fail: examine failures, fix if branch-related
    4. `make check` — verify resolution
    5. If pre-existing failure: `bd create -t bug --title "Pre-existing: <failure>"`
    6. When `make check` passes, say "I'm done"

    ## Rules

    - You are NOT an approval gate — tests passing IS the approval
    - If conflict is complex, escalate (say you can't resolve it)
    - Never skip tests — if they fail, either fix or file a bug
  PROMPT
}
