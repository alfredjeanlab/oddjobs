# Team lead agent that spawns teammates for parallel work.
#
# Claude plans first, then creates a team to parallelize implementation.
#
# Examples:
#   oj run team "Refactor the storage layer and add compression support"
#   oj run team "Build REST API with auth, validation, and tests"

command "team" {
  args = "<instructions>"
  run  = { job = "team" }
}

job "team" {
  name = "team: ${var.instructions}"
  vars = ["instructions"]

  step "work" {
    run = { agent = "leader" }
  }
}

agent "leader" {
  run     = "claude --model opus --dangerously-skip-permissions"
  on_idle = "done"
  on_dead = "done"

  env = {
    CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS = "1"
  }

  session "tmux" {
    color = "magenta"
    title = "Claude Team"
  }

  prime = {
    startup = <<-SHELL
      echo '## Reminders'
      echo '- Use AskUserQuestion frequently to clarify requirements and get user input'
      echo '- You have agent teams enabled — use them for parallel work'
    SHELL
    clear = <<-SHELL
      echo '## Reminders'
      echo '- Use AskUserQuestion frequently to clarify requirements and get user input'
      echo '- Even now that the plan is approved, continue asking questions when uncertain'
      echo '- You have agent teams enabled — spawn teammates for parallel implementation'
    SHELL
  }

  prompt = <<-PROMPT
    ${var.instructions}

    IMPORTANT: Before implementing anything, create a thorough plan:
    1. Use EnterPlanMode to switch to planning mode
    2. Explore the codebase to understand the current state
    3. Design your approach, using AskUserQuestion to clarify ambiguities
    4. Call ExitPlanMode to present your plan for approval

    You have agent teams enabled. After your plan is approved, use them to
    parallelize implementation:
    - Create a team and spawn teammates for independent pieces of work
    - Use delegate mode (Shift+Tab) to focus on coordination
    - Give each teammate clear, self-contained tasks with enough context
    - Avoid assigning teammates to the same files
    - Require plan approval for teammates doing complex or risky changes
    - Monitor progress and redirect teammates as needed

    Throughout your work, use AskUserQuestion frequently to:
    - Clarify ambiguous requirements
    - Confirm design decisions before implementing
    - Get input on trade-offs
  PROMPT
}
