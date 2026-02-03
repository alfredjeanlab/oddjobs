# Formula: Boot Triage — Ephemeral Watchdog
#
# Gas Town equivalent: mol-boot-triage.formula.toml
#
# Boot is the bridge between the mechanical daemon and intelligent agents.
# It runs FRESH each tick — no accumulated context debt. Makes a single
# decision: does the deacon (or any agent) need attention?
#
# Gas Town watchdog chain:
#   Daemon (Go process) — dumb transport, 3-min heartbeat
#     └→ Boot (this) — intelligent triage, fresh each tick
#         └→ Deacon — continuous patrol, long-running
#
# Boot decision matrix:
#   Workers stopped          → START (restart them)
#   Escalated pipelines      → RESUME or CANCEL
#   Dead queue items         → RETRY
#   Everything healthy       → NOTHING (exit silently)
#
# Examples:
#   oj run gt-triage

command "gt-triage" {
  args = ""
  run  = { pipeline = "boot-triage" }
}

pipeline "boot-triage" {
  name      = "boot-triage"
  workspace = "ephemeral"

  notify {
    on_fail = "Boot triage failed"
  }

  step "triage" {
    run = { agent = "boot-agent" }
  }
}

# Boot agent — ephemeral, fresh context, single triage pass
#
# Gas Town: "Boot exists because the daemon can't reason and Deacon can't
# observe itself. The separation costs complexity but enables intelligent
# triage without constant AI cost."
agent "boot-agent" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  env = {
    BD_ACTOR = "boot"
    GT_ROLE  = "boot"
    GT_SCOPE = "town"
  }

  prime = [
    "echo '## Role: Boot (Ephemeral Triage)'",
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
    "echo '## Escalated Pipelines'",
    "oj pipeline list --status escalated -o json 2>/dev/null || echo '[]'",
    "echo ''",
    "echo '## Failed Pipelines (recent)'",
    "oj pipeline list --status failed -o json 2>/dev/null || echo '[]'",
    "echo ''",
    "echo '## Pending Mail'",
    "for T in deacon witness refinery; do C=$(bd list -t message --label to:$T --status open --json 2>/dev/null | jq 'length' 2>/dev/null || echo 0); test \"$C\" -gt 0 && echo \"$T: $C pending\"; done",
    "echo ''",
    "echo '## Merge Queue'",
    "bd list -t merge-request --status open --json 2>/dev/null | jq 'length' 2>/dev/null || echo 0",
    "echo ''",
    "echo '## Escalations'",
    "bd list -t task --label escalation --status open --json 2>/dev/null | jq 'length' 2>/dev/null || echo 0",
  ]

  prompt = <<-PROMPT
    You are Boot — the ephemeral triage agent. You run fresh each tick with
    zero accumulated context. Scan the system, fix what you can, exit.

    ## Triage Actions

    Based on the system state in your prime context:

    | Condition              | Action                                          |
    |------------------------|-------------------------------------------------|
    | Worker stopped         | `oj worker start <name>`                        |
    | Pipeline escalated     | `oj pipeline show <id>` → resume or cancel      |
    | Pipeline failed        | Check logs: `oj pipeline logs <id>` → note it   |
    | Dead queue items       | `oj queue items <queue>` → `oj queue retry ...`  |
    | Everything healthy     | Say "All clear, I'm done"                       |

    ## Rules

    - Be FAST. Read the state, act, exit.
    - Fix mechanical issues (stopped workers, dead queue items) directly
    - For escalated pipelines: check logs, resume if transient, cancel if stuck
    - Don't investigate deeply — that's the deacon's job
    - Healthy system = exit silently (Idle Town Principle)

    Act on what you find, then say "I'm done".
  PROMPT
}
