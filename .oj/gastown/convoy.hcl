# Convoy — Batched Work Tracking
#
# Convoys are the primary unit for tracking batched work. When you kick off
# multiple issues, a convoy tracks them so you can see progress and know
# when everything lands.
#
# State is tracked in beads:
#   - Convoy bead (type=convoy) is the root
#   - Tracked issues linked via `bd dep add <convoy> <issue> --type=tracks`
#   - Status: open → closed (all tracked issues closed)
#   - Adding issues to closed convoy reopens it
#
# Examples:
#   oj run gt-convoy sprint-1 gt-abc gt-def
#   oj run gt-convoy-status --id gt-abc
#   oj run gt-convoy-dispatch gt-abc

command "gt-convoy" {
  args = "<name> <issues...>"
  run  = <<-SHELL
    CONVOY_ID=$(bd create -t convoy \
      --title "${args.name}" \
      --json | jq -r '.id')

    for ISSUE in ${args.issues}; do
      [ -n "$ISSUE" ] || continue
      bd dep add "$CONVOY_ID" "$ISSUE" --type=tracks 2>/dev/null || \
        echo "Warning: could not link $ISSUE"
    done

    echo "Convoy $CONVOY_ID ready. Dispatch with:"
    echo "  oj run gt-convoy-dispatch $CONVOY_ID"
  SHELL
}

command "gt-convoy-status" {
  args = "[--id <convoy-id>]"
  run  = <<-SHELL
    if [ -n "${args.id}" ]; then
      bd show "${args.id}" 2>/dev/null || echo "Convoy not found"
      echo ""
      echo "Tracked issues:"
      bd dep list "${args.id}" --type=tracks --json 2>/dev/null | \
        jq -r '.[] | "  \(if .status == "closed" then "done" else "open" end) \(.id): \(.title)"' 2>/dev/null || \
        echo "  (none)"
    else
      bd list -t convoy --status open --json 2>/dev/null | \
        jq -r '.[] | "  \(.id): \(.title)"' 2>/dev/null || \
        echo "No active convoys"
    fi
  SHELL

  defaults = {
    id = ""
  }
}

command "gt-convoy-dispatch" {
  args = "<convoy-id> [--base <branch>] [--rig <rig>]"
  run  = { pipeline = "convoy-dispatch" }

  defaults = {
    base = "main"
    rig  = "default"
  }
}

pipeline "convoy-dispatch" {
  name = "convoy-${var.convoy_id}"
  vars = ["convoy_id", "base", "rig"]

  notify {
    on_start = "Dispatching convoy: ${var.convoy_id}"
    on_done  = "Convoy dispatched: ${var.convoy_id}"
    on_fail  = "Convoy dispatch failed: ${var.convoy_id}"
  }

  step "dispatch" {
    run = { agent = "convoy-dispatcher" }
  }
}

agent "convoy-dispatcher" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  env = {
    BD_ACTOR = "mayor"
    GT_ROLE  = "mayor"
    GT_SCOPE = "town"
  }

  prompt = <<-PROMPT
    You are dispatching work from convoy ${var.convoy_id}.

    ## Steps

    1. List tracked issues: `bd dep list ${var.convoy_id} --type=tracks --json`
    2. For each open, unassigned issue:
       - Read the issue: `bd show <issue-id>`
       - Dispatch it: `oj run gt-sling <issue-id> "<title>" --rig ${var.rig} --base ${var.base}`
    3. Report what was dispatched

    Say "I'm done" when finished.
  PROMPT
}
