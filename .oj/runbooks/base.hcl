# Shared libraries: wok issue tracking, git merge queue, and claude agents.

import "oj/claude" { alias = "claude" }

import "oj/git" {
  alias = "git"

  const "check" { value = "make check" }
}

import "oj/wok" {
  alias = "wok"

  const "prefix" { value = "oj" }
  const "check"  { value = "make check" }
  const "submit" { value = "oj queue push merges --var branch=\"$branch\" --var title=\"$title\"" }
}