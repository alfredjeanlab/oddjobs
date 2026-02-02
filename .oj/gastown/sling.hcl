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
#   4. Spawn polecat pipeline in ephemeral workspace
#   5. Optionally create a convoy for tracking
#
# Usage:
#   oj run gt-sling <issue> <instructions> [--base <branch>]

command "gt-sling" {
  args = "<issue> <instructions> [--base <branch>] [--rig <rig>]"
  run  = { pipeline = "sling" }

  defaults = {
    base = "main"
    rig  = "default"
  }
}

pipeline "sling" {
  vars      = ["issue", "instructions", "base", "rig"]
  workspace = "ephemeral"

  # Create work bead, molecule steps, and spawn polecat workspace
  step "provision" {
    run = <<-SHELL
      # If issue looks like an existing bead ID (has a dash), show it;
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
      POLECAT_NAME="${var.issue}-${workspace.nonce}"
      bd update "$BEAD_ID" \
        --status in_progress \
        --assignee "${var.rig}/polecats/$POLECAT_NAME"

      # Write state for downstream steps
      echo "$BEAD_ID" > .bead-id
      echo "$POLECAT_NAME" > .polecat-name
      echo "Slung $BEAD_ID to ${var.rig}/polecats/$POLECAT_NAME"

      # Spawn polecat workspace
      REPO="$(git -C ${invoke.dir} rev-parse --show-toplevel)"
      BRANCH="polecat/$POLECAT_NAME"
      git -C "$REPO" worktree add -b "$BRANCH" "${workspace.root}/work" origin/${var.base}
    SHELL
    on_done = { step = "execute" }
  }

  # Run the polecat-work formula
  step "execute" {
    run = { agent = "polecat-worker" }
    on_done = { step = "done" }
  }

  # Completion: push, create MR, clean up
  step "done" {
    run = <<-SHELL
      cd "${workspace.root}/work"
      REPO="$(git -C ${invoke.dir} rev-parse --show-toplevel)"
      BEAD_ID="$(cat ${workspace.root}/.bead-id)"
      POLECAT_NAME="$(cat ${workspace.root}/.polecat-name)"
      BRANCH="$(git branch --show-current)"

      git add -A
      git diff --cached --quiet || git commit -m "feat(${var.issue}): ${var.instructions}"

      git -C "$REPO" push origin "$BRANCH"

      # Create merge-request bead (enters the refinery queue)
      bd create -t merge-request \
        --title "MR: ${var.instructions}" \
        --description "Branch: $BRANCH\nIssue: $BEAD_ID\nBase: ${var.base}" \
        --labels "branch:$BRANCH,issue:$BEAD_ID,base:${var.base},rig:${var.rig}" \
        --json > /dev/null

      # Send POLECAT_DONE mail to witness
      bd create -t message \
        --title "POLECAT_DONE $POLECAT_NAME" \
        --description "Exit: MERGED\nIssue: $BEAD_ID\nBranch: $BRANCH" \
        --labels "from:${var.rig}/polecats/$POLECAT_NAME,to:witness,msg-type:polecat-done"

      bd close "$BEAD_ID" --reason "Submitted to merge queue"
    SHELL
  }
}

# The polecat worker agent — discovers and executes molecule steps
agent "polecat-worker" {
  run      = "claude --dangerously-skip-permissions"
  cwd      = "${workspace.root}/work"
  on_idle  = { action = "nudge", message = "You have work. Check `bd ready` for your next step. Execute it. Say 'I'm done' when all steps are complete." }
  on_dead  = { action = "gate", run = "make check" }
  on_error = { action = "escalate" }

  env = {
    BD_ACTOR         = "${var.rig}/polecats/${var.issue}-${workspace.nonce}"
    GIT_AUTHOR_NAME  = "${var.issue}-${workspace.nonce}"
    GT_ROLE          = "polecat"
    GT_RIG           = "${var.rig}"
    GT_POLECAT       = "${var.issue}-${workspace.nonce}"
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

    ## The Propulsion Principle

    Work was placed on your hook. The system trusts you will BEGIN IMMEDIATELY.

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
