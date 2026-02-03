# Formula: Shutdown Dance — Health Check Enforcement
#
# Gas Town equivalent: mol-shutdown-dance.formula.toml
#
# Dogs execute shutdown dances: a deterministic state machine that
# interrogates unresponsive agents and either pardons or terminates them.
#
# State machine: WARRANT → INTERROGATE → EVALUATE → PARDON | EXECUTE
#
# Three attempts with escalating timeouts:
#   Attempt 1: 60s  (cumulative: 1 min)
#   Attempt 2: 120s (cumulative: 3 min)
#   Attempt 3: 240s (cumulative: 7 min)
#
# If the target responds (shows activity in beads or oj status), it's pardoned.
# If no response after 3 attempts, the warrant is executed.
#
# Gas Town insight: Dogs are NOT Claude sessions — they're lightweight
# state machines. We match that here: pure shell steps, no AI agents.
#
# Usage:
#   oj run gt-shutdown-dance <target> <reason>

command "gt-shutdown-dance" {
  args = "<target> <reason>"
  run  = { pipeline = "shutdown-dance" }
}

pipeline "shutdown-dance" {
  name = "shutdown-${var.target}"
  vars = ["target", "reason"]

  notify {
    on_done = "Shutdown dance done: ${var.target}"
    on_fail = "Shutdown dance failed: ${var.target}"
  }

  # Record the warrant in beads
  step "warrant" {
    run = <<-SHELL
      WARRANT_ID=$(bd create -t task \
        --title "Shutdown warrant: ${var.target}" \
        --description "Reason: ${var.reason}\nTarget: ${var.target}\nFiled: $(date -u +%Y-%m-%dT%H:%M:%SZ)" \
        --labels "warrant,target:${var.target}" \
        --json 2>/dev/null | jq -r '.id' 2>/dev/null || echo "warrant-unknown")

      echo "$WARRANT_ID" > .warrant-id
    SHELL
    on_done = { step = "interrogate-1" }
  }

  # Attempt 1: wait 60s then check for activity
  step "interrogate-1" {
    run = <<-SHELL
      sleep 60
      bd list --assignee "${var.target}" --json 2>/dev/null | jq -e 'length > 0' >/dev/null 2>&1 && exit 0
      oj status 2>/dev/null | grep -q "${var.target}" && exit 0
      exit 1
    SHELL
    on_done = { step = "pardon" }
    on_fail = { step = "interrogate-2" }
  }

  # Attempt 2: wait 120s then check again
  step "interrogate-2" {
    run = <<-SHELL
      sleep 120
      bd list --assignee "${var.target}" --json 2>/dev/null | jq -e 'length > 0' >/dev/null 2>&1 && exit 0
      oj status 2>/dev/null | grep -q "${var.target}" && exit 0
      exit 1
    SHELL
    on_done = { step = "pardon" }
    on_fail = { step = "interrogate-3" }
  }

  # Attempt 3: wait 240s — final check
  step "interrogate-3" {
    run = <<-SHELL
      sleep 240
      bd list --assignee "${var.target}" --json 2>/dev/null | jq -e 'length > 0' >/dev/null 2>&1 && exit 0
      oj status 2>/dev/null | grep -q "${var.target}" && exit 0
      exit 1
    SHELL
    on_done = { step = "pardon" }
    on_fail = { step = "execute" }
  }

  # Target responded — cancel warrant
  step "pardon" {
    run = <<-SHELL
      WARRANT_ID="$(cat .warrant-id 2>/dev/null || echo unknown)"
      bd close "$WARRANT_ID" --reason "Pardoned: target responsive" 2>/dev/null || true

      bd create -t message \
        --title "DOG_DONE: PARDONED ${var.target}" \
        --description "Warrant: $WARRANT_ID\nOutcome: pardoned" \
        --labels "from:dog,to:deacon,msg-type:dog-done" 2>/dev/null || true
    SHELL
  }

  # All attempts exhausted — terminate
  step "execute" {
    run = <<-SHELL
      WARRANT_ID="$(cat .warrant-id 2>/dev/null || echo unknown)"
      bd close "$WARRANT_ID" --reason "Executed: target unresponsive" 2>/dev/null || true

      bd create -t task \
        --title "Warrant executed: ${var.target}" \
        --description "Reason: ${var.reason}\nWarrant: $WARRANT_ID\nOutcome: executed after 3 attempts" \
        --labels "escalation,severity:medium,source:dog" 2>/dev/null || true

      bd create -t message \
        --title "DOG_DONE: EXECUTED ${var.target}" \
        --description "Warrant: $WARRANT_ID\nOutcome: executed\nReason: ${var.reason}" \
        --labels "from:dog,to:deacon,msg-type:dog-done" 2>/dev/null || true
    SHELL
  }
}
