# Formula: Polecat Work — The Canonical Worker Lifecycle
#
# Gas Town equivalent: mol-polecat-work.formula.toml
#
# This is THE formula that polecats execute. Every polecat runs this job.
# Steps: init → work → submit
#
# Key Gas Town principles:
#   - Propulsion: work on hook = execute immediately
#   - Self-cleaning: push → submit MR → exit → cease to exist
#   - Steps discovered via `bd ready`, closed via `bd close`
#   - Session cycling is normal (prime re-injects context)
#   - Rebase-as-work: conflicts spawn fresh polecats, never "sent back"

job "polecat-work" {
  name      = "${var.bug.id}"
  vars      = ["bug"]
  workspace = "folder"
  on_cancel = { step = "cancel" }
  on_fail   = { step = "reopen" }

  locals {
    repo   = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    branch = "polecat/${var.bug.id}-${workspace.nonce}"
    title  = "fix: ${var.bug.title}"
    actor  = "polecat/${var.bug.id}"
  }

  notify {
    on_start = "Polecat: ${var.bug.title}"
    on_done  = "Polecat done: ${var.bug.title}"
    on_fail  = "Polecat failed: ${var.bug.title}"
  }

  # Initialize workspace (worktree from shared repo)
  step "init" {
    run = <<-SHELL
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}" HEAD

      # Store state for agent discovery
      echo "${var.bug.id}" > .hook-bead
      echo "${local.branch}" > .branch-name

      # Hook the bead to this polecat
      bd update "${var.bug.id}" --status in_progress \
        --assignee "${local.actor}" 2>/dev/null || true
    SHELL
    on_done = { step = "work" }
  }

  # The polecat executes: discovers steps, implements, tests
  step "work" {
    run     = { agent = "polecat" }
    on_done = { step = "submit" }
  }

  # Submit: push branch, create MR bead, send POLECAT_DONE mail
  step "submit" {
    run = <<-SHELL
      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      git -C "${local.repo}" push origin "${local.branch}"

      # Create merge-request bead (enters the refinery queue)
      MR_ID=$(bd create -t merge-request \
        --title "MR: ${var.bug.title}" \
        --description "Branch: ${local.branch}\nIssue: ${var.bug.id}\nBase: main" \
        --labels "branch:${local.branch},issue:${var.bug.id},base:main" \
        --json 2>/dev/null | jq -r '.id' 2>/dev/null || echo "mr-unknown")

      # Send POLECAT_DONE to witness (the mail protocol)
      bd create -t message \
        --title "POLECAT_DONE" \
        --description "Exit: MERGED\nIssue: ${var.bug.id}\nMR: $MR_ID\nBranch: ${local.branch}" \
        --labels "from:polecat,to:witness,msg-type:polecat-done" 2>/dev/null || true

      bd close "${var.bug.id}" --reason "Submitted MR $MR_ID" 2>/dev/null || true
    SHELL
    on_done = { step = "cleanup" }
  }

  step "cancel" {
    run     = "bd close \"${var.bug.id}\" --reason 'Job cancelled' 2>/dev/null || true"
    on_done = { step = "cleanup" }
  }

  step "reopen" {
    run     = "bd reopen \"${var.bug.id}\" --reason 'Job failed' 2>/dev/null || true"
    on_done = { step = "cleanup" }
  }

  step "cleanup" {
    run = "git -C \"${local.repo}\" worktree remove --force \"${workspace.root}\" 2>/dev/null || true"
  }
}

# The polecat agent — the actual Claude session that does the work
#
# Gas Town: polecats have three layers:
#   Session (this agent) — ephemeral, cycles on handoff/crash
#   Sandbox (workspace)  — persists until submit
#   Slot (identity)      — attribution, persists in beads
agent "polecat" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "nudge", message = "Check `bd ready` for your next step. Execute it and `bd close` it when done. Say 'I'm done' when all work is complete." }
  on_dead  = { action = "gate", run = "make check" }

  env = {
    BD_ACTOR        = "${local.actor}"
    GIT_AUTHOR_NAME = "${var.bug.id}"
    GT_ROLE         = "polecat"
  }

  # Prime: inject context at session start (Gas Town's gt prime)
  prime = <<-SHELL
    echo '## Role: Polecat (Ephemeral Worker)'
    echo 'You are a polecat. Execute your hook. Done means gone.'
    echo ''
    echo '## Hook'
    BEAD_ID="$(cat .hook-bead 2>/dev/null || echo '')"
    if [ -n "$BEAD_ID" ]; then
      bd show "$BEAD_ID" 2>/dev/null || echo "Hook bead: $BEAD_ID (could not load)"
      echo ''
      echo '## Ready Steps'
      bd ready --parent="$BEAD_ID" --json 2>/dev/null || echo '[]'
    else
      echo 'WARNING: No hook bead found. Check .hook-bead file.'
    fi
    echo ''
    echo '## Advice'
    bd advice list --for="polecat/$BEAD_ID" 2>/dev/null || echo 'No advice.'
    echo ''
    echo '## Git'
    git branch --show-current 2>/dev/null || true
    git log --oneline -3 2>/dev/null || true
    git status --short 2>/dev/null | head -10
  SHELL

  prompt = <<-PROMPT
    You are a polecat — an ephemeral worker. Execute the work on your hook.

    ## Step Discovery (from beads)

    Your work is tracked as molecule steps. Discover them:

    ```bash
    BEAD_ID="$(cat .hook-bead)"
    bd ready --parent="$BEAD_ID" --json    # Find next step
    bd show <step-id>                       # Read step instructions
    # ... execute the step ...
    bd close <step-id>                      # Mark step complete
    ```

    If no molecule steps exist, work directly from the hook bead's description.

    ## Implementation

    1. Read the hook bead and understand the task
    2. Find and execute each ready step (or work directly if no steps)
    3. Write or update tests for changed code
    4. Run `make check` to verify
    5. Commit changes with descriptive messages
    6. When all work is complete, say "I'm done"

    ## Rules

    - Close steps in real-time (each closure is a timestamped record)
    - Do NOT push to main — your branch goes to the merge queue
    - Do NOT wait for merge results — submit and exit
    - If stuck, file a bug: `bd create -t bug --title "Stuck: <reason>"`
  PROMPT
}
