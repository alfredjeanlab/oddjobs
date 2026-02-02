# Formula: Witness Patrol — Per-Rig Agent Health Monitor
#
# Gas Town equivalent: mol-witness-patrol.formula.toml
#
# The Witness monitors polecat health within a rig. It processes mail
# (POLECAT_DONE, MERGED) and detects stalled/zombie polecats.
#
# Patrol cycle:
#   1. inbox-check — process mail (POLECAT_DONE, MERGED, HELP)
#   2. health-scan — check active polecats for stalled/zombie state
#   3. cleanup — nuke completed polecat worktrees
#
# Three polecat states (NO idle state):
#   Working — actively doing work
#   Stalled — session stopped mid-work (crashed, never nudged)
#   Zombie  — completed work but failed to exit cleanly
#
# Usage:
#   oj run gt-witness-patrol [--rig <rig>]

command "gt-witness-patrol" {
  args = "[--rig <rig>]"
  run  = { pipeline = "witness-patrol" }

  defaults = {
    rig = "default"
  }
}

pipeline "witness-patrol" {
  vars      = ["rig"]
  workspace = "ephemeral"

  # Process the witness inbox
  step "inbox" {
    run = <<-SHELL
      # Process POLECAT_DONE messages
      DONE_MSGS=$(bd list -t message \
        --label "to:witness" \
        --label "msg-type:polecat-done" \
        --status open --json 2>/dev/null || echo '[]')

      echo "$DONE_MSGS" | jq -r '.[].id' 2>/dev/null | while read -r MSG_ID; do
        [ -n "$MSG_ID" ] && bd close "$MSG_ID" --reason "Processed by witness" 2>/dev/null || true
      done

      # Process MERGED messages
      MERGED_MSGS=$(bd list -t message \
        --label "to:witness" \
        --label "msg-type:merged" \
        --status open --json 2>/dev/null || echo '[]')

      echo "$MERGED_MSGS" | jq -r '.[].id' 2>/dev/null | while read -r MSG_ID; do
        [ -n "$MSG_ID" ] && bd close "$MSG_ID" --reason "Processed by witness" 2>/dev/null || true
      done
    SHELL
    on_done = { step = "health-scan" }
  }

  # Check health of active polecats
  step "health-scan" {
    run = { agent = "witness-agent" }
  }
}

# Witness agent — monitors polecat health
agent "witness-agent" {
  run      = "claude --dangerously-skip-permissions"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  env = {
    BD_ACTOR = "${var.rig}/witness"
    GT_ROLE  = "witness"
    GT_RIG   = "${var.rig}"
    GT_SCOPE = "rig"
  }

  prime = <<-SHELL
    echo '## Role: Witness (Health Monitor)'
    echo "Rig: ${var.rig}"
    echo ''
    echo '## Active Pipelines'
    oj status 2>/dev/null || echo '  No active pipelines'
    echo ''
    echo '## Pending Mail'
    bd list -t message --label "to:witness" --status open --json 2>/dev/null | \
      jq -r '.[] | "  \(.title)"' 2>/dev/null || \
      echo '  No pending mail'
    echo ''
    echo '## Active Pipelines'
    oj status 2>/dev/null || echo '  Could not check oj status'
  SHELL

  prompt = <<-PROMPT
    You are the Witness — health monitor for rig ${var.rig}.

    ## Patrol Cycle

    1. Check `oj status` for running pipelines
    2. For each, assess health:
       - **Working**: recent activity → leave alone
       - **Stalled**: no activity, should be working → note for nudge
       - **Zombie**: done but didn't clean up → note for cleanup
    3. Check for pending mail you haven't processed
    4. Report findings concisely

    ## Rules

    - Do NOT interrupt working agents
    - Do NOT force session cycles (agents self-manage)
    - Stalled ≠ idle. There IS no idle state.
    - If an agent exists without work, something is broken.

    Report findings and say "I'm done".
  PROMPT
}
