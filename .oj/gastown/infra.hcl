# Infrastructure — Shared Queues and Workers
#
# The backbone of the system. These queues and workers connect the formulas
# together into a working whole.
#
# Beads is the state layer. These queues poll beads for work.

# ---------------------------------------------------------------------------
# Merge Queue — backed by merge-request beads
# ---------------------------------------------------------------------------
# Polecats submit MRs via `bd create -t merge-request`. The refinery worker
# polls for pending MRs and processes them sequentially.

queue "merge-requests" {
  type = "external"
  list = "bd list -t merge-request --status open --json"
  take = "bd update ${item.id} --status in_progress --assignee ${BD_ACTOR:-refinery}"
}

worker "refinery" {
  source      = { queue = "merge-requests" }
  handler     = { job = "refinery-patrol" }
  concurrency = 1
}

# ---------------------------------------------------------------------------
# Bug Queue — backed by bug beads, same as original bugfix.hcl pattern
# ---------------------------------------------------------------------------

queue "bugs" {
  type = "external"
  list = "bd list -t bug --status open --no-assignee --json"
  take = "bd update ${item.id} --status in_progress --assignee ${BD_ACTOR:-polecat}"
}

worker "bugfix" {
  source      = { queue = "bugs" }
  handler     = { job = "polecat-work" }
  concurrency = 1
}

# ---------------------------------------------------------------------------
# Ready Work Queue — any unassigned open work beads
# ---------------------------------------------------------------------------
# The deacon can dispatch from this queue when polecats are available.

queue "ready-work" {
  type = "external"
  list = "bd ready --json"
  take = "bd update ${item.id} --status in_progress --assignee ${BD_ACTOR:-polecat}"
}

# ---------------------------------------------------------------------------
# Mail Queue — unread messages for a specific agent
# ---------------------------------------------------------------------------
# Each agent role polls its own inbox. The witness checks for POLECAT_DONE,
# MERGED, etc. The refinery checks for MERGE_READY.

# Note: Gas Town routes witness mail per-rig (to:<rig>/witness). In oj, we
# match both rig-qualified and bare "witness" labels. If GT_RIG is set in the
# environment, the rig-specific inbox is used; otherwise falls back to "witness".
queue "witness-inbox" {
  type = "external"
  list = "bd list -t message --label to:witness --status open --json"
  take = "bd update ${item.id} --status in_progress"
}

queue "deacon-inbox" {
  type = "external"
  list = "bd list -t message --label to:deacon --status open --json"
  take = "bd update ${item.id} --status in_progress"
}
