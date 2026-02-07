# Interactive planning agent with human-in-the-loop decisions.
#
# Launches Claude in plan mode — the agent explores the codebase, creates
# a plan for approval, then implements. Uses AskUserQuestion frequently
# to clarify requirements.
#
# Usage:
#   oj run claude "Implement user authentication with OAuth"

command "claude" {
  args = "<instructions>"
  run  = { job = "claude" }
}

job "claude" {
  name = "${var.instructions}"
  vars = ["instructions"]

  step "work" {
    run = { agent = "claude" }
  }
}

agent "claude" {
  run     = "claude --model opus --dangerously-skip-permissions"
  on_idle = "done"
  on_dead = "done"

  session "tmux" {
    color = "green"
    title = "Claude"
  }

  prime = {
    startup = <<-SHELL
      echo '## Reminders'
      echo '- Use AskUserQuestion frequently to clarify requirements and get user input'
    SHELL
    clear = <<-SHELL
      echo '## Reminders'
      echo '- Use AskUserQuestion frequently to clarify requirements and get user input'
      echo '- Even now that the plan is approved, continue asking questions when uncertain'
    SHELL
  }

  prompt = <<-PROMPT
    ${var.instructions}

    IMPORTANT: Before implementing anything, create a thorough plan:
    1. Use EnterPlanMode to switch to planning mode
    2. Explore the codebase to understand the current state
    3. Design your approach, using AskUserQuestion to clarify ambiguities
    4. Call ExitPlanMode to present your plan for approval

    Throughout your work, use AskUserQuestion frequently to:
    - Clarify ambiguous requirements
    - Confirm design decisions before implementing
    - Get input on trade-offs
  PROMPT
}
