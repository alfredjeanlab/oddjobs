# Shared libraries: wok issue tracking, git merge queue, and claude agents.

import "oj/claude" { alias = "claude" }

import "oj/wok" {
  const "prefix" { value = "oj" }
  const "check"  { value = "make check" }
  const "submit" { value = "oj queue push merges --var branch=\"$branch\" --var title=\"$title\"" }
}

import "oj/git" {
  const "check" { value = "make check" }
}
