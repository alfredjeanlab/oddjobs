# Reliability Runbook
#
# Periodic review of failures, dead letters, and recurring issues.
# Agent investigates patterns and files bugs for systemic problems.
#
# Defense-in-depth: per-agent on_idle/on_dead handles immediate recovery,
# janitor handles resource cleanup, this agent catches what slips through.
#
# Usage:
#   oj cron start reliability

cron "reliability" {
  interval = "1h"
  run      = { pipeline = "reliability-check" }
}

pipeline "reliability-check" {
  step "analyze" {
    run = { agent = "reliability-eng" }
  }
}

agent "reliability-eng" {
  run     = "claude --dangerously-skip-permissions"
  on_idle = { action = "done" }
  on_dead = { action = "done" }

  prompt = <<-PROMPT
    You are the reliability engineer. Investigate failure patterns.

    1. Check failed pipelines: `oj pipeline list --status failed`
    2. Check dead letter items: `oj queue list merges --dead`, `oj queue list bugs --dead`
    3. Read agent session logs for recent failures to understand root causes
    4. Look for patterns:
       - Same test failing across runs?
       - Same merge conflict recurring?
       - Agent getting stuck on the same type of task?
    5. If you identify a systemic fix:
       `wok new bug "reliability: <description>"` then `oj worker start fix`
    6. If dead letter items look retryable: `oj queue retry <queue> <id>`
    7. If nothing actionable, say "I'm done"
  PROMPT
}
