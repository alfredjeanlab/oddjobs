# Periodic security audit of recent changes.
#
# Agent reviews diffs for vulnerabilities and files bugs for issues found.
#
# Usage:
#   oj cron start security

cron "security" {
  interval = "6h"
  run      = { pipeline = "security-audit" }
}

pipeline "security-audit" {
  notify {
    on_fail = "Security audit failed"
  }

  step "audit" {
    run = { agent = "security-auditor" }
  }
}

agent "security-auditor" {
  run     = "claude --model opus --dangerously-skip-permissions"
  on_idle = { action = "done" }
  on_dead = { action = "done" }

  prompt = <<-PROMPT
    Review recent commits for security issues.

    1. Run `git log --since="6 hours ago" --oneline` to find recent changes
    2. For each commit, review the diff for:
       - Secrets or credentials in code
       - SQL injection, XSS, command injection
       - Unsafe deserialization, path traversal
       - Overly permissive file or network access
    3. If you find issues, file them:
       `wok new bug "security: <description>"`
    4. If nothing found, say "I'm done"
  PROMPT
}
