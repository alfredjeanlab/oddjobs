# Formula: Deacon Patrol — Town-Level Orchestration
#
# Gas Town equivalent: mol-deacon-patrol.formula.toml
#
# The Deacon is the town-level orchestrator. Its patrol cycle handles:
#   - Inbox processing (mail from witnesses, mayor, escalations)
#   - Health scanning (are witnesses and refineries alive?)
#   - Convoy completion checks
#   - Ready work discovery
#
# "Idle Town Principle": be silent when healthy and idle. Don't flood logs.
#
# Usage:
#   oj run gt-deacon-patrol

command "gt-deacon-patrol" {
  args = ""
  run  = { pipeline = "deacon-patrol" }
}

pipeline "deacon-patrol" {
  workspace = "ephemeral"

  step "patrol" {
    run = <<-SHELL
      # --- Inbox: process deacon mail ---
      MSGS=$(bd list -t message --label "to:deacon" --status open --json 2>/dev/null || echo '[]')
      COUNT=$(echo "$MSGS" | jq 'length' 2>/dev/null || echo 0)

      if [ "$COUNT" -gt 0 ]; then
        echo "$MSGS" | jq -r '.[] | .title' 2>/dev/null
        echo "$MSGS" | jq -r '.[].id' 2>/dev/null | while read -r ID; do
          [ -n "$ID" ] && bd close "$ID" --reason "Processed by deacon" 2>/dev/null || true
        done
      fi

      # --- Health scan: check for unacked escalations ---
      STALE=$(bd list -t task --label escalation --status open --json 2>/dev/null | \
        jq '[.[] | select(.labels | index("acknowledged:false"))] | length' 2>/dev/null || echo 0)
      test "$STALE" -gt 0 && echo "unacked-escalations: $STALE"

      oj status 2>/dev/null || true

      # --- Convoy check: auto-close completed convoys ---
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
          fi
        done
      fi

      # --- Ready work: report undispatched items ---
      READY=$(bd ready --json 2>/dev/null || echo '[]')
      READY_COUNT=$(echo "$READY" | jq 'length' 2>/dev/null || echo 0)

      if [ "$READY_COUNT" -gt 0 ]; then
        echo "$READY" | jq -r '.[] | "\(.id): \(.title)"' 2>/dev/null
      fi
    SHELL
  }
}
