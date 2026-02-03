# Formula: Deacon Patrol — Town-Level Orchestration
#
# Gas Town equivalent: mol-deacon-patrol.formula.toml
#
# The Deacon is the town-level orchestrator. Its patrol cycle handles:
#   - Inbox processing (mail from witnesses, mayor, escalations)
#   - Health scanning (are witnesses and refineries alive?)
#   - Convoy completion checks
#   - Ready work discovery and dispatch
#
# "Idle Town Principle": be silent when healthy and idle. Don't flood logs.
#
# Examples:
#   oj run gt-deacon-patrol

command "gt-deacon-patrol" {
  args = ""
  run  = { pipeline = "deacon-patrol" }
}

pipeline "deacon-patrol" {
  name      = "deacon-patrol"
  workspace = "ephemeral"

  notify {
    on_fail = "Deacon patrol failed"
  }

  # Process deacon inbox (pure shell — close processed messages)
  step "inbox" {
    run = <<-SHELL
      MSGS=$(bd list -t message --label "to:deacon" --status open --json 2>/dev/null || echo '[]')
      COUNT=$(echo "$MSGS" | jq 'length' 2>/dev/null || echo 0)

      if [ "$COUNT" -gt 0 ]; then
        echo "Processing $COUNT deacon messages:"
        echo "$MSGS" | jq -r '.[] | "  \(.id): \(.title)"' 2>/dev/null
        echo "$MSGS" | jq -r '.[].id' 2>/dev/null | while read -r ID; do
          [ -n "$ID" ] && bd close "$ID" --reason "Processed by deacon" 2>/dev/null || true
        done
      fi
    SHELL
    on_done = { step = "convoy-check" }
  }

  # Auto-close completed convoys (pure shell — deterministic)
  step "convoy-check" {
    run = <<-SHELL
      CONVOYS=$(bd list -t convoy --status open --json 2>/dev/null || echo '[]')
      CV_COUNT=$(echo "$CONVOYS" | jq 'length' 2>/dev/null || echo 0)

      if [ "$CV_COUNT" -gt 0 ]; then
        echo "$CONVOYS" | jq -r '.[].id' 2>/dev/null | while read -r CV_ID; do
          [ -z "$CV_ID" ] && continue
          TRACKED=$(bd dep list "$CV_ID" --type=tracks --json 2>/dev/null || echo '[]')
          TOTAL=$(echo "$TRACKED" | jq 'length' 2>/dev/null || echo 0)
          CLOSED=$(echo "$TRACKED" | jq '[.[] | select(.status == "closed")] | length' 2>/dev/null || echo 0)
          if [ "$TOTAL" -gt 0 ] && [ "$TOTAL" = "$CLOSED" ]; then
            bd close "$CV_ID" --reason "All tracked issues closed" 2>/dev/null || true
            echo "Closed convoy $CV_ID (all $TOTAL issues done)"
          fi
        done
      fi
    SHELL
    on_done = { step = "patrol" }
  }

  # Agent-driven patrol: health scan, escalation handling, work dispatch
  step "patrol" {
    run = { agent = "deacon-agent" }
  }
}

agent "deacon-agent" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  env = {
    BD_ACTOR = "deacon"
    GT_ROLE  = "deacon"
    GT_SCOPE = "town"
  }

  prime = [
    "echo '## Role: Deacon (Town Orchestrator)'",
    "echo ''",
    "echo '## oj CLI Reference'",
    "oj --help 2>/dev/null",
    "echo ''",
    "echo '## System Status'",
    "oj status 2>/dev/null || echo 'No active work'",
    "echo ''",
    "echo '## Workers'",
    "oj worker list -o json 2>/dev/null || echo '[]'",
    "echo ''",
    "echo '## Running Pipelines'",
    "oj pipeline list --status running -o json 2>/dev/null || echo '[]'",
    "echo ''",
    "echo '## Escalated Pipelines'",
    "oj pipeline list --status escalated -o json 2>/dev/null || echo '[]'",
    "echo ''",
    "echo '## Queues'",
    "oj queue list -o json 2>/dev/null || echo '[]'",
    "echo ''",
    "echo '## Unacked Escalations'",
    "bd list -t task --label escalation --status open --json 2>/dev/null | jq -r '.[] | \"  \\(.id): \\(.title)\"' 2>/dev/null || echo '  (none)'",
    "echo ''",
    "echo '## Ready Work (undispatched)'",
    "bd ready --unassigned --json 2>/dev/null | jq -r '.[] | \"  \\(.id): \\(.title)\"' 2>/dev/null || echo '  (none)'",
    "echo ''",
    "echo '## Open Convoys'",
    "bd list -t convoy --status open --json 2>/dev/null | jq -r '.[] | \"  \\(.id): \\(.title)\"' 2>/dev/null || echo '  (none)'",
  ]

  prompt = <<-PROMPT
    You are the Deacon — town-level orchestrator. You patrol the system,
    fix issues, and dispatch ready work.

    ## Patrol Duties

    1. **Workers**: ensure all workers are running
       - `oj worker start <name>` for any stopped workers

    2. **Escalated pipelines**: investigate and resolve
       - `oj pipeline show <id>` and `oj pipeline logs <id>` to understand the issue
       - `oj pipeline resume <id>` if the error is transient
       - `oj pipeline cancel <id>` if unrecoverable
       - File a bug if the failure reveals a systemic issue:
         `bd create -t bug --title "..."`

    3. **Escalation beads**: acknowledge open escalations
       - `bd show <id>` to understand
       - `bd close <id> --reason "Acknowledged: <action taken>"` after handling

    4. **Ready work**: dispatch undispatched items
       - For each ready, unassigned bead: `oj run gt-sling <bead-id> "<title>"`

    5. **Queue health**: check for dead items
       - `oj queue items <queue-name>` to inspect
       - `oj queue retry <queue> <item-id>` for retriable failures

    ## Rules

    - Idle Town Principle: if everything is healthy, exit silently
    - Don't duplicate work — check if something is already being handled
    - Be concise: report what you found and what you did
    - Don't investigate code — that's polecat work

    Act on what you find, then say "I'm done".
  PROMPT
}
