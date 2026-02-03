# Start — Town Startup Chain
#
# Starts the Gas Town watchdog chain: daemon ensures all agents are alive,
# boot triages deacon health, deacon patrols town, witnesses monitor rigs.
#
# Gas Town equivalent: `gt start`
#
# Startup order:
#   1. Ensure beads database exists
#   2. Start refinery worker (merge queue processor)
#   3. Start witness patrol (per-rig health monitor)
#   4. Start deacon patrol (town-level orchestrator)
#   5. Boot triage runs periodically to verify deacon health
#
# Usage:
#   oj run gt-start [--rig <rig>]

command "gt-start" {
  args = "[--rig <rig>]"
  run  = <<-SHELL
    # Check bd is available
    command -v bd >/dev/null 2>&1 || { echo "Error: bd (beads) not found in PATH"; exit 1; }

    # Ensure custom types are configured (Gas Town types)
    bd config set types.custom "molecule,gate,convoy,merge-request,slot,agent,role,rig,event,message" 2>/dev/null || true

    # Ensure agent beads exist for core roles
    for ROLE in mayor deacon witness refinery; do
      AGENT_ID="hq-$ROLE"
      bd show "$AGENT_ID" >/dev/null 2>&1 || \
        bd create -t agent --id "$AGENT_ID" --title "$ROLE agent" --labels "role:$ROLE" 2>/dev/null || true
    done

    # Start the background workers
    oj worker start refinery 2>/dev/null || true
    oj worker start bugfix 2>/dev/null || true
  SHELL

  defaults = {
    rig = "default"
  }
}

command "gt-status" {
  args = ""
  run  = "oj status 2>/dev/null; echo '---'; for ROLE in mayor deacon witness refinery; do bd agent show \"hq-$ROLE\" 2>/dev/null || true; done"
}

command "gt-stop" {
  args = ""
  run  = <<-SHELL
    echo "Stopping town..."

    # Send shutdown mail to all agents
    for ROLE in witness refinery deacon; do
      bd create -t message \
        --title "LIFECYCLE:Shutdown" \
        --description "Town shutdown requested" \
        --labels "from:mayor,to:$ROLE,msg-type:lifecycle" 2>/dev/null || true
    done

    echo "Shutdown messages sent."
    echo "Workers will exit after current work completes."
  SHELL
}
