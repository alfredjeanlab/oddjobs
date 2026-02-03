# Periodic cleanup of stale resources.
#
# Simple shell steps — no agent needed.
#
# Usage:
#   oj cron start janitor

cron "janitor" {
  interval = "30m"
  run      = { pipeline = "cleanup" }
}

pipeline "cleanup" {
  notify {
    on_fail = "Janitor failed"
  }

  step "worktrees" {
    run     = "oj workspace prune"
    on_done = { step = "sessions" }
  }

  step "sessions" {
    run     = "oj session prune"
    on_done = { step = "logs" }
  }

  step "logs" {
    run = "find .oj/logs -type f -mtime +30 -delete 2>/dev/null; true"
  }
}
