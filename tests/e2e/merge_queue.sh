#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="/tmp/oj-merge-test"
rm -rf "$TEST_DIR"
trap 'rm -rf "$TEST_DIR"' EXIT

# --- Setup test repo ---
git init "$TEST_DIR"
cd "$TEST_DIR"
git checkout -b main

cat > Makefile <<'EOF'
.PHONY: check
check:
	@echo ok
EOF

echo "hello" > hello.txt
git add -A
git commit -m "initial commit"

# --- Setup runbook ---
mkdir -p .oj/runbooks/merge
cat > .oj/runbooks/merge/local.hcl <<'RUNBOOK'
queue "merges" {
  type     = "persisted"
  vars     = ["branch", "title", "base"]
  defaults = { base = "main" }
}

worker "merge" {
  source      = { queue = "merges" }
  run = { job = "merge" }
  concurrency = 1
}

job "merge" {
  vars = ["mr"]

  step "init" {
    run = <<-SHELL
      git checkout ${var.mr.base}
      git merge --no-edit ${var.mr.branch}
    SHELL
    on_done = { step = "check" }
  }

  step "check" {
    run     = "make check"
    on_done = { step = "done" }
  }

  step "done" {
    run = "echo merge complete"
  }
}
RUNBOOK

# --- Create feature branch ---
git checkout -b test-feature
echo "world" >> hello.txt
git add hello.txt
git commit -m "feat: add world"
git checkout main

# --- Start daemon ---
export OJ_STATE_DIR="$TEST_DIR/.oj/state"
oj daemon start

oj queue push merges '{"branch": "test-feature", "title": "test: add world"}'

oj worker start merge

# --- Poll for completion ---
# Following the project's no-sleep policy: poll with timeout, not arbitrary sleeps
TIMEOUT=30
ELAPSED=0
while [ $ELAPSED -lt $TIMEOUT ]; do
  if oj job list 2>/dev/null | grep -q "completed"; then
    break
  fi
  sleep 1  # polling interval, not synchronization
  ELAPSED=$((ELAPSED + 1))
done

if [ $ELAPSED -ge $TIMEOUT ]; then
  echo "FAIL: merge job did not complete within ${TIMEOUT}s"
  oj job list
  oj daemon stop
  exit 1
fi

# --- Verify merge result ---
git checkout main
if grep -q "world" hello.txt; then
  echo "PASS: feature branch merged into main"
else
  echo "FAIL: expected 'world' in hello.txt on main"
  git log --oneline -5
  oj daemon stop
  exit 1
fi

oj daemon stop
echo "All checks passed."
