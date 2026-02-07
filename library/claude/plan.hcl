# Interactive planning agent with human-in-the-loop decisions.
#
# Claude explores the codebase, creates a plan for approval, then implements.
# Uses AskUserQuestion frequently to clarify requirements.
#
# Examples:
#   oj run plan "Implement user authentication with OAuth"
#   oj run plan "Add dark mode theme support"

command "plan" {
  args = "<instructions>"
  run  = { job = "plan" }
}

job "plan" {
  name = "${var.instructions}"
  vars = ["instructions"]

  step "work" {
    run = { agent = "planner" }
  }
}

agent "planner" {
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
      echo '- When done, commit your changes (do NOT stash, reset, or discard — other agents may be editing concurrently)'
    SHELL
    clear = <<-SHELL
      echo '## Reminders'
      echo '- Use AskUserQuestion frequently to clarify requirements and get user input'
      echo '- Even now that the plan is approved, continue asking questions when uncertain'
      echo '- When done, commit your changes (do NOT stash, reset, or discard — other agents may be editing concurrently)'
    SHELL
    compact = <<-SHELL
      echo '## Reminders'
      echo '- Use AskUserQuestion frequently to clarify requirements and get user input'
      echo '- Even now that the plan is approved, continue asking questions when uncertain'
      echo '- When done, commit your changes (do NOT stash, reset, or discard — other agents may be editing concurrently)'
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

    When finished, commit your changes. Do NOT use git stash, git reset, or discard changes — other agents may be editing the repository concurrently.
  PROMPT
}
