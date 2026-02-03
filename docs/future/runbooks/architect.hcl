# Architect Runbook
#
# Periodic review of recent changes for architectural quality.
# Agent analyzes patterns, design drift, duplication, and code quality
# regressions. Writes findings to a review note rather than filing issues —
# not every observation needs a ticket.
#
# Usage:
#   oj cron start architect

cron "architect" {
  interval = "24h"
  run      = { pipeline = "architect-review" }
}

pipeline "architect-review" {
  notify {
    on_fail = "Architect review failed"
  }

  step "review" {
    run = { agent = "architect" }
  }
}

agent "architect" {
  run     = "claude --model opus --dangerously-skip-permissions"
  on_idle = { action = "done" }
  on_dead = { action = "done" }

  prompt = <<-PROMPT
    You are the architect. Review recent changes for quality and design coherence.

    1. Run `git log --since="24 hours ago" --oneline` to find recent changes
    2. For each commit, review the diff looking for:
       - Architectural drift from established patterns
       - Code duplication or missed abstractions
       - Design issues (wrong layer, leaky abstractions, tight coupling)
       - Code quality regressions (inconsistent naming, unclear control flow)
       - Public API surface creep (unnecessary exports)
    3. Write a summary of your findings:
       `wok note <project-issue> "architect: <findings>"` or print them
    4. If you find something significant enough to warrant action, file it:
       `wok new chore "architect: <description>"`
    5. If nothing noteworthy, say "I'm done"
  PROMPT
}
