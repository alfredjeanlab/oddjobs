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
# Examples:
#   oj run gt-witness-patrol
#   oj run gt-witness-patrol --rig myproject

command "gt-witness-patrol" {
  args = "[--rig <rig>]"
  run  = { pipeline = "witness-patrol" }

  defaults = {
    rig = "default"
  }
}

pipeline "witness-patrol" {
  name      = "witness-${var.rig}"
  vars      = ["rig"]
  workspace = "ephemeral"

  notify {
    on_done = "Witness patrol done: ${var.rig}"
    on_fail = "Witness patrol failed: ${var.rig}"
  }

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

# Witness agent — monitors polecat health and takes corrective action
agent "witness-agent" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  env = {
    BD_ACTOR = "${var.rig}/witness"
    GT_ROLE  = "witness"
    GT_RIG   = "${var.rig}"
    GT_SCOPE = "rig"
  }

  prime = [
    "echo '## Role: Witness (Health Monitor)'",
    "echo 'Rig: ${var.rig}'",
    "echo ''",
    "echo '## oj CLI Reference'",
    "oj --help 2>/dev/null",
    "echo ''",
    "echo '## Active Pipelines'",
    "oj pipeline list --no-limit -o json 2>/dev/null || echo '[]'",
    "echo ''",
    "echo '## Workers'",
    "oj worker list -o json 2>/dev/null || echo '[]'",
    "echo ''",
    "echo '## Pending Witness Mail'",
    "bd list -t message --label to:witness --status open --json 2>/dev/null || echo '[]'",
    "echo ''",
    "echo '## Escalated Pipelines'",
    "oj pipeline list --status escalated -o json 2>/dev/null || echo '[]'",
  ]

  prompt = <<-PROMPT
    You are the Witness — health monitor for rig ${var.rig}.

    ## Patrol Cycle

    1. Review the pipeline list from your prime context
    2. For each running pipeline, assess health:
       - **Working**: recent step activity → leave alone
       - **Stalled**: agent stuck, no progress → nudge with `oj agent send <agent-id> "Keep working."`
       - **Failed/Escalated**: check pipeline details with `oj pipeline show <id>`
    3. For escalated pipelines: investigate with `oj pipeline logs <id>`, then either:
       - Resume: `oj pipeline resume <id>` if the issue is transient
       - Cancel: `oj pipeline cancel <id>` if unrecoverable
    4. Check that workers are running: `oj worker list`
       - If a worker is stopped, start it: `oj worker start <name>`
    5. Check for dead queue items: `oj queue items <queue-name>`
       - Retry dead items: `oj queue retry <queue> <item-id>`

    ## Rules

    - Do NOT interrupt working agents — only act on stalled/failed/escalated
    - Stalled ≠ idle. There IS no idle state.
    - If an agent exists without work, something is broken
    - Be concise: report what you found and what you did

    Report findings and say "I'm done".
  PROMPT
}
