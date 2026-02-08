# Dependency blocking for GitHub Issues.
#
# Issues can declare dependencies via "Blocked by: #2, #5, #14" in the body.
# The `blocked` label prevents workers from picking up the issue. An unblock
# cron runs every 60s to detect when all deps are resolved and removes the label.
#
# A dep is resolved when CLOSED and no open PR is still pending merge for it.
# This prevents premature unblocking when a dep issue is closed before its
# PR actually merges.
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
        body="$(printf '%s\n\nBlocked by:%s' "$body" "$refs")"
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
    open_prs=$(gh pr list --json number,body --jq '.' 2>/dev/null || echo '[]')
    gh issue list --label blocked --state open --json number,body | jq -c '.[]' | while read -r obj; do
      num=$(echo "$obj" | jq -r .number)
      deps=$(echo "$obj" | jq -r '.body' | grep -iE 'Blocked by:?' | grep -oE '#[0-9]+' | grep -oE '[0-9]+' || true)
      if [ -z "$deps" ]; then
        gh issue edit "$num" --remove-label blocked
        echo "Unblocked #$num (no deps)"
        continue
      fi
      all_resolved=true
      for dep in $deps; do
        state=$(gh issue view "$dep" --json state -q .state 2>/dev/null)
        if [ "$state" != "CLOSED" ]; then
          all_resolved=false
          break
        fi
        has_open_pr=$(echo "$open_prs" | jq --arg dep "$dep" '[.[] | select(.body | contains("Closes #" + $dep))] | length')
        if [ "$has_open_pr" -gt 0 ]; then
          all_resolved=false
          break
        fi
      done
      if [ "$all_resolved" = true ]; then
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
      open_prs=$(gh pr list --json number,body --jq '.' 2>/dev/null || echo '[]')
      gh issue list --label blocked --state open --json number,body | jq -c '.[]' | while read -r obj; do
        num=$(echo "$obj" | jq -r .number)
        deps=$(echo "$obj" | jq -r '.body' | grep -iE 'Blocked by:?' | grep -oE '#[0-9]+' | grep -oE '[0-9]+' || true)
        if [ -z "$deps" ]; then
          gh issue edit "$num" --remove-label blocked
          echo "Unblocked #$num (no deps)"
          continue
        fi
        all_resolved=true
        for dep in $deps; do
          state=$(gh issue view "$dep" --json state -q .state 2>/dev/null)
          if [ "$state" != "CLOSED" ]; then
            all_resolved=false
            break
          fi
          has_open_pr=$(echo "$open_prs" | jq --arg dep "$dep" '[.[] | select(.body | contains("Closes #" + $dep))] | length')
          if [ "$has_open_pr" -gt 0 ]; then
            all_resolved=false
            break
          fi
        done
        if [ "$all_resolved" = true ]; then
          gh issue edit "$num" --remove-label blocked
          echo "Unblocked #$num"
        fi
      done
    SHELL
  }
}
