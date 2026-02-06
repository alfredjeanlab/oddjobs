# Shared libraries: wok issue tracking and git merge queue.

import "oj/wok" {
  const "prefix" { value = "oj" }
  const "check"  { value = "make check" }
  const "submit" { value = "oj queue push merges --var branch=\"$branch\" --var title=\"$title\"" }
}

import "oj/git" {
  const "check" { value = "make check" }
}
