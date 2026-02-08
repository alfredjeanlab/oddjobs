# Dependency blocking for GitHub Issues.
#
# Issues can declare dependencies via "Blocked by: #2, #5, #14" in the body.
# The `blocked` label prevents workers from picking up the issue. An unblock
# cron runs every 60s to detect when all deps close and removes the label.
#
# All issue commands (fix, chore, epic) accept --blocked to set deps at
# creation time. They set `_blocked` then inline the apply_blocked const.

const "blocked" {
  default = <<-SHELL
    if [ -n "$_blocked" ]; then
      labels="$labels,blocked"
      nums=$(echo "$_blocked" | tr ',' ' ')
      refs=""
      for n in $nums; do refs="$refs #$n"; done
      if [ -n "$body" ]; then
        body="$body\\n\\nBlocked by:$refs"
      else
        body="Blocked by:$refs"
      fi
    fi
  SHELL
}

# Check all blocked issues and remove label when all deps are resolved.
#
# Examples:
#   oj run unblock
command "unblock" {
  run = <<-SHELL
    gh issue list --label blocked --state open --json number,body | jq -c '.[]' | while read -r obj; do
      num=$(echo "$obj" | jq -r .number)
      deps=$(echo "$obj" | jq -r '.body' | grep -iE 'Blocked by:?' | grep -oE '#[0-9]+' | grep -oE '[0-9]+' || true)
      if [ -z "$deps" ]; then
        gh issue edit "$num" --remove-label blocked
        echo "Unblocked #$num (no deps)"
        continue
      fi
      all_closed=true
      for dep in $deps; do
        state=$(gh issue view "$dep" --json state -q .state 2>/dev/null)
        if [ "$state" != "CLOSED" ]; then
          all_closed=false
          break
        fi
      done
      if [ "$all_closed" = true ]; then
        gh issue edit "$num" --remove-label blocked
        echo "Unblocked #$num"
      fi
    done
  SHELL
}

cron "unblock" {
  interval = "60s"
  run      = { job = "unblock" }
}

job "unblock" {
  name = "unblock"

  step "check" {
    run = <<-SHELL
      gh issue list --label blocked --state open --json number,body | jq -c '.[]' | while read -r obj; do
        num=$(echo "$obj" | jq -r .number)
        deps=$(echo "$obj" | jq -r '.body' | grep -iE 'Blocked by:?' | grep -oE '#[0-9]+' | grep -oE '[0-9]+' || true)
        if [ -z "$deps" ]; then
          gh issue edit "$num" --remove-label blocked
          echo "Unblocked #$num (no deps)"
          continue
        fi
        all_closed=true
        for dep in $deps; do
          state=$(gh issue view "$dep" --json state -q .state 2>/dev/null)
          if [ "$state" != "CLOSED" ]; then
            all_closed=false
            break
          fi
        done
        if [ "$all_closed" = true ]; then
          gh issue edit "$num" --remove-label blocked
          echo "Unblocked #$num"
        fi
      done
    SHELL
  }
}
