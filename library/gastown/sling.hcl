# Sling — Work Dispatch
#
# The primary way work enters the system. Creates a bead, instantiates a
# molecule (workflow steps), and spawns a polecat to execute it.
#
# Gas Town equivalent: `gt sling <issue> <rig>`
#
# Flow:
#   1. Create issue bead in beads (or use existing bead ID)
#   2. Instantiate polecat-work molecule (creates step beads)
#   3. Hook the molecule to a polecat (set assignee + hook label)
#   4. Spawn polecat job in folder workspace
#   5. Optionally create a convoy for tracking
#
# Examples:
#   oj run gt-sling auth-fix "Fix the auth bug"
#   oj run gt-sling gt-abc "Existing issue" --base develop

command "gt-sling" {
  args = "<issue> <instructions> [--base <branch>] [--rig <rig>]"
  run  = { job = "sling" }

  defaults = {
    base = "main"
    rig  = "default"
  }
}

job "sling" {
  name      = "${var.issue}"
  vars      = ["issue", "instructions", "base", "rig"]
  workspace = "folder"
  on_cancel = { step = "cleanup" }
  on_fail   = { step = "reopen" }

  locals {
    repo    = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
    polecat = "${var.issue}-${workspace.nonce}"
    branch  = "polecat/${var.issue}-${workspace.nonce}"
    actor   = "${var.rig}/polecats/${var.issue}-${workspace.nonce}"
    title   = "feat(${var.issue}): ${var.instructions}"
  }

  notify {
    on_start = "Slung: ${var.issue}"
    on_done  = "Polecat done: ${var.issue}"
    on_fail  = "Polecat failed: ${var.issue}"
  }

  # Create work bead, molecule steps, and spawn polecat workspace
  step "provision" {
    run = <<-SHELL
      # If issue looks like an existing bead ID (has a dash), use it;
      # otherwise create a new task bead
      if echo "${var.issue}" | grep -q '-'; then
        BEAD_ID="${var.issue}"
        echo "Using existing bead: $BEAD_ID"
      else
        BEAD_ID=$(bd create -t task \
          --title "${var.instructions}" \
          --labels "rig:${var.rig}" \
          --json | jq -r '.id')
        echo "Created bead: $BEAD_ID"
      fi

      # Create molecule: instantiate polecat-work steps as child beads
      for STEP in "load-context" "implement" "test" "submit"; do
        bd create -t task \
          --title "Step: $STEP" \
          --parent "$BEAD_ID" \
          --labels "molecule-step,step:$STEP" \
          --json > /dev/null
      done

      # Hook the work to a polecat identity
      bd update "$BEAD_ID" \
        --status in_progress \
        --assignee "${local.actor}"

      # Write bead ID for downstream steps
      echo "$BEAD_ID" > .bead-id
      echo "Slung $BEAD_ID to ${local.actor}"

      # Spawn polecat workspace
      git -C "${local.repo}" worktree add -b "${local.branch}" "${workspace.root}/work" origin/${var.base}
    SHELL
    on_done = { step = "execute" }
  }

  # Run the polecat-work formula
  step "execute" {
    run     = { agent = "polecat-worker" }
    on_done = { step = "submit" }
  }

  # Completion: push, create MR, clean up
  step "submit" {
    run = <<-SHELL
      cd "${workspace.root}/work"
      BEAD_ID="$(cat ${workspace.root}/.bead-id)"

      git add -A
      git diff --cached --quiet || git commit -m "${local.title}"
      git -C "${local.repo}" push origin "${local.branch}"

      # Create merge-request bead (enters the refinery queue)
      bd create -t merge-request \
        --title "MR: ${var.instructions}" \
        --description "Branch: ${local.branch}\nIssue: $BEAD_ID\nBase: ${var.base}" \
        --labels "branch:${local.branch},issue:$BEAD_ID,base:${var.base},rig:${var.rig}" \
        --json > /dev/null

      # Send POLECAT_DONE mail to witness
      bd create -t message \
        --title "POLECAT_DONE ${local.polecat}" \
        --description "Exit: MERGED\nIssue: $BEAD_ID\nBranch: ${local.branch}" \
        --labels "from:${local.actor},to:witness,msg-type:polecat-done"

      bd close "$BEAD_ID" --reason "Submitted to merge queue"
    SHELL
    on_done = { step = "cleanup" }
  }

  step "reopen" {
    run = <<-SHELL
      BEAD_ID="$(cat ${workspace.root}/.bead-id 2>/dev/null || echo '')"
      test -n "$BEAD_ID" && bd reopen "$BEAD_ID" --reason "Sling job failed" 2>/dev/null || true
    SHELL
    on_done = { step = "cleanup" }
  }

  step "cleanup" {
    run = "git -C \"${local.repo}\" worktree remove --force \"${workspace.root}/work\" 2>/dev/null || true"
  }
}

# The polecat worker agent — discovers and executes molecule steps
agent "polecat-worker" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  cwd      = "${workspace.root}/work"
  on_idle  = { action = "nudge", message = "You have work. Check `bd ready` for your next step. Execute it. Say 'I'm done' when all steps are complete." }
  on_dead  = { action = "gate", run = "make check" }

  env = {
    BD_ACTOR         = "${local.actor}"
    GIT_AUTHOR_NAME  = "${local.polecat}"
    GT_ROLE          = "polecat"
    GT_RIG           = "${var.rig}"
    GT_POLECAT       = "${local.polecat}"
  }

  # Prime: inject context at session start (Gas Town's gt prime equivalent)
  prime = [
    "echo '## Identity'",
    "echo \"BD_ACTOR=$BD_ACTOR\"",
    "echo '## Hook'",
    "cat ${workspace.root}/.bead-id 2>/dev/null && bd show $(cat ${workspace.root}/.bead-id) 2>/dev/null || echo 'No hook found'",
    "echo '## Ready Steps'",
    "bd ready --parent=$(cat ${workspace.root}/.bead-id 2>/dev/null) --json 2>/dev/null || echo '[]'",
    "echo '## Advice'",
    "bd advice list --for=$BD_ACTOR 2>/dev/null || echo 'No advice.'",
    "echo '## Git'",
    "git branch --show-current",
    "git log --oneline -3",
  ]

  prompt = <<-PROMPT
    You are a polecat — an ephemeral worker. You have ONE job: execute the steps
    on your hook, then exit. There is no idle state. Done means gone.

    ## Workflow

    Your work is tracked as molecule steps in beads. Discover them:

    1. Find your hook: the bead ID is in your prime context above
    2. Find ready steps: `bd ready --parent=<bead-id> --json`
    3. Read step instructions: `bd show <step-id>`
    4. Execute the step
    5. Close the step: `bd close <step-id>`
    6. Repeat from step 2 until no more ready steps
    7. Run `make check` to verify everything passes
    8. Commit your changes
    9. Say "I'm done"

    ## Instructions

    ${var.instructions}

    ## Rules

    - Close steps IN REAL TIME — mark in_progress before starting, close after
    - Do NOT push to main — your branch goes to the merge queue
    - Do NOT wait for merge results — submit and exit
    - If stuck: `bd create -t bug --title "Stuck: <reason>"` and escalate
  PROMPT
}
