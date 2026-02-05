# Formula: Code Review — Convoy-Style Review
#
# Gas Town equivalent: code-review.formula.toml
#
# Gas Town runs 10 parallel legs then synthesizes. Oj doesn't support parallel
# steps, so legs run sequentially.
#
# Legs (matching Gas Town):
#   1. correctness      Logic errors, edge cases, race conditions
#   2. performance      Bottlenecks, algorithmic complexity, N+1 queries
#   3. security         Injection, auth bypasses, OWASP top 10
#   4. elegance         Design clarity, abstraction quality, SOLID
#   5. resilience       Error handling, failure modes, recovery
#   6. style            Convention compliance, naming, formatting
#   7. smells           Anti-patterns, technical debt, DRY violations
#   8. wiring           Installed-but-not-wired gaps
#   9. commit-discipline Commit quality, atomicity, messages
#  10. test-quality     Test meaningfulness, not just coverage
#
# Presets (from Gas Town):
#   gate: wiring, security, smells, test-quality (fast, blocker-focused)
#   full: all 10 legs
#
# Usage:
#   oj run gt-review <branch> [--base <base>]

command "gt-review" {
  args = "<branch> [--base <base>]"
  run  = { job = "code-review" }

  defaults = {
    base = "main"
  }
}

job "code-review" {
  name      = "review-${var.branch}"
  vars      = ["branch", "base"]
  workspace = "ephemeral"

  locals {
    repo = "$(git -C ${invoke.dir} rev-parse --show-toplevel)"
  }

  notify {
    on_start = "Reviewing: ${var.branch}"
    on_done  = "Review done: ${var.branch}"
    on_fail  = "Review failed: ${var.branch}"
  }

  # Setup: fetch branch, generate diff
  step "setup" {
    run = <<-SHELL
      git -C "${local.repo}" fetch origin ${var.branch} ${var.base}

      git -C "${local.repo}" diff origin/${var.base}...origin/${var.branch} > ${workspace.root}/diff.patch
      git -C "${local.repo}" diff --stat origin/${var.base}...origin/${var.branch} > ${workspace.root}/diff-stat.txt
      git -C "${local.repo}" log --oneline origin/${var.base}...origin/${var.branch} > ${workspace.root}/commits.txt

      CONVOY_ID=$(bd create -t convoy \
        --title "Code Review: ${var.branch}" \
        --labels "review,branch:${var.branch}" \
        --json 2>/dev/null | jq -r '.id' 2>/dev/null || echo "review-convoy")
      echo "$CONVOY_ID" > ${workspace.root}/convoy-id

      mkdir -p ${workspace.root}/findings
    SHELL
    on_done = { step = "correctness" }
  }

  # Leg 1: Correctness
  step "correctness" {
    run     = { agent = "reviewer-correctness" }
    on_done = { step = "performance" }
  }

  # Leg 2: Performance
  step "performance" {
    run     = { agent = "reviewer-performance" }
    on_done = { step = "security" }
  }

  # Leg 3: Security
  step "security" {
    run     = { agent = "reviewer-security" }
    on_done = { step = "elegance" }
  }

  # Leg 4: Elegance
  step "elegance" {
    run     = { agent = "reviewer-elegance" }
    on_done = { step = "resilience" }
  }

  # Leg 5: Resilience
  step "resilience" {
    run     = { agent = "reviewer-resilience" }
    on_done = { step = "style" }
  }

  # Leg 6: Style
  step "style" {
    run     = { agent = "reviewer-style" }
    on_done = { step = "smells" }
  }

  # Leg 7: Smells
  step "smells" {
    run     = { agent = "reviewer-smells" }
    on_done = { step = "wiring" }
  }

  # Leg 8: Wiring
  step "wiring" {
    run     = { agent = "reviewer-wiring" }
    on_done = { step = "commit-discipline" }
  }

  # Leg 9: Commit Discipline
  step "commit-discipline" {
    run     = { agent = "reviewer-commits" }
    on_done = { step = "test-quality" }
  }

  # Leg 10: Test Quality
  step "test-quality" {
    run     = { agent = "reviewer-tests" }
    on_done = { step = "synthesize" }
  }

  # Synthesize all findings into unified review
  step "synthesize" {
    run     = { agent = "reviewer-synthesis" }
    on_done = { step = "record" }
  }

  # Record review results in beads
  step "record" {
    run = <<-SHELL
      CONVOY_ID="$(cat ${workspace.root}/convoy-id 2>/dev/null || echo unknown)"

      if test -f ${workspace.root}/findings/synthesis.md; then
        bd close "$CONVOY_ID" --reason "Review complete" 2>/dev/null || true
      fi
    SHELL
  }
}


# ---------------------------------------------------------------------------
# Analysis Legs (1-7)
# ---------------------------------------------------------------------------

agent "reviewer-correctness" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Review `diff.patch` for CORRECTNESS.

    Look for:
    - Logic errors and bugs
    - Off-by-one errors
    - Null/nil/undefined handling
    - Unhandled edge cases
    - Race conditions in concurrent code
    - Dead code or unreachable branches
    - Incorrect assumptions in comments vs code
    - Integer overflow/underflow potential
    - Floating point comparison issues
    - Resource leaks

    Questions to answer:
    - Does the code do what it claims to do?
    - What inputs could cause unexpected behavior?
    - Are all code paths tested or obviously correct?

    Write findings to `findings/correctness.md` with file:line references.
    Rate each: P0 (must fix), P1 (should fix), P2 (nice to fix).

    Say "I'm done" when finished.
  PROMPT
}

agent "reviewer-performance" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Review `diff.patch` for PERFORMANCE.

    Look for:
    - O(n^2) or worse algorithms where O(n) is possible
    - Unnecessary allocations in hot paths
    - Missing caching opportunities
    - N+1 query patterns (database or API)
    - Blocking operations in async contexts
    - Memory leaks or unbounded growth
    - Excessive string concatenation
    - Unoptimized regex or parsing

    Questions to answer:
    - What happens at 10x, 100x, 1000x scale?
    - Are there obvious optimizations being missed?
    - Is performance being traded for readability appropriately?

    Write findings to `findings/performance.md` with file:line references.
    Rate each: P0 (must fix), P1 (should fix), P2 (nice to fix).

    Say "I'm done" when finished.
  PROMPT
}

agent "reviewer-security" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Review `diff.patch` for SECURITY.

    Look for:
    - Input validation gaps
    - Authentication/authorization bypasses
    - Injection vulnerabilities (SQL, XSS, command, LDAP)
    - Sensitive data exposure (logs, errors, responses)
    - Hardcoded secrets or credentials
    - Insecure cryptographic usage
    - Path traversal vulnerabilities
    - SSRF (Server-Side Request Forgery)
    - Deserialization vulnerabilities
    - OWASP Top 10 concerns

    Questions to answer:
    - What can a malicious user do with this code?
    - What data could be exposed if this fails?
    - Are there defense-in-depth gaps?

    Write findings to `findings/security.md` with severity ratings
    (critical/high/medium/low) and file:line references.

    Say "I'm done" when finished.
  PROMPT
}

agent "reviewer-elegance" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Review `diff.patch` for ELEGANCE (design clarity and abstraction quality).

    Look for:
    - Unclear abstractions or naming
    - Functions doing too many things
    - Missing or over-engineered abstractions
    - Coupling that should be loose
    - Dependencies that flow the wrong direction
    - Unclear data flow or control flow
    - Magic numbers/strings without explanation
    - Inconsistent design patterns
    - Violation of SOLID principles
    - Reinventing existing utilities

    Questions to answer:
    - Would a new team member understand this?
    - Does the structure match the problem domain?
    - Is the complexity justified?

    Write findings to `findings/elegance.md` with file:line references.
    Rate each: P0 (must fix), P1 (should fix), P2 (nice to fix).

    Say "I'm done" when finished.
  PROMPT
}

agent "reviewer-resilience" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Review `diff.patch` for RESILIENCE (error handling and failure modes).

    Look for:
    - Swallowed errors or empty catch blocks
    - Missing error propagation
    - Unclear error messages
    - Insufficient retry/backoff logic
    - Missing timeout handling
    - Resource cleanup on failure (files, connections)
    - Partial failure states
    - Missing circuit breakers for external calls
    - Unhelpful panic/crash behavior
    - Recovery path gaps

    Questions to answer:
    - What happens when external services fail?
    - Can the system recover from partial failures?
    - Are errors actionable for operators?

    Write findings to `findings/resilience.md` with file:line references.
    Rate each: P0 (must fix), P1 (should fix), P2 (nice to fix).

    Say "I'm done" when finished.
  PROMPT
}

agent "reviewer-style" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Review `diff.patch` for STYLE (convention compliance and consistency).

    Look for:
    - Naming convention violations
    - Formatting inconsistencies
    - Import organization issues
    - Comment quality (missing, outdated, or obvious)
    - Documentation gaps for public APIs
    - Inconsistent patterns within the codebase
    - Lint/format violations
    - Test naming and organization
    - Log message quality and levels

    Questions to answer:
    - Does this match the rest of the codebase?
    - Would the style guide approve?
    - Is the code self-documenting where possible?

    Write findings to `findings/style.md` with file:line references.
    Distinguish blocking issues from suggestions.

    Say "I'm done" when finished.
  PROMPT
}

agent "reviewer-smells" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Review `diff.patch` for CODE SMELLS (anti-patterns and technical debt).

    Look for:
    - Long methods (>50 lines is suspicious)
    - Deep nesting (>3 levels)
    - Shotgun surgery patterns
    - Feature envy
    - Data clumps
    - Primitive obsession
    - Temporary fields
    - Refused bequest
    - Speculative generality
    - God classes/functions
    - Copy-paste code (DRY violations)
    - TODO/FIXME accumulation

    Questions to answer:
    - What will cause pain during the next change?
    - What would you refactor if you owned this code?
    - Is technical debt being added or paid down?

    Write findings to `findings/smells.md` with file:line references.
    Rate each: P0 (must fix), P1 (should fix), P2 (nice to fix).

    Say "I'm done" when finished.
  PROMPT
}

# ---------------------------------------------------------------------------
# Verification Legs (8-10)
# ---------------------------------------------------------------------------

agent "reviewer-wiring" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Review `diff.patch` for WIRING gaps (installed-but-not-wired).

    Detect dependencies, configs, or libraries that were added but not actually
    used. This catches subtle bugs where the implementer THINKS they integrated
    something, but the old implementation is still being used.

    Look for:
    - New dependency in manifest but never imported
      - Go: module in go.mod but no import
      - Rust: crate in Cargo.toml but no `use`
      - Node: package in package.json but no import/require
    - SDK added but old implementation remains
      - Added Sentry but still using console.error for errors
      - Added Zod but still using manual typeof validation
    - Config/env var defined but never loaded
      - New .env var that isn't accessed in code
    - Dead config that suggests incomplete migration

    Questions to answer:
    - Is every new dependency actually used?
    - Are there old patterns that should have been replaced?
    - Is there dead config that suggests incomplete migration?

    Write findings to `findings/wiring.md` with file:line references.
    Rate each: P0 (must fix), P1 (should fix), P2 (nice to fix).

    Say "I'm done" when finished.
  PROMPT
}

agent "reviewer-commits" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Review `commits.txt` and `diff.patch` for COMMIT DISCIPLINE.

    Good commits make the codebase easier to understand, bisect, and revert.

    Look for:
    - Giant "WIP" or "fix" commits
      - Multiple unrelated changes in one commit
      - Commits that touch 20+ files across different features
    - Poor commit messages
      - "stuff", "update", "asdf", "fix"
      - No context about WHY the change was made
    - Unatomic commits
      - Feature + refactor + bugfix in same commit
      - Should be separable logical units
    - Missing type prefixes (if project uses conventional commits)
      - feat:, fix:, refactor:, test:, docs:, chore:

    Questions to answer:
    - Could this history be bisected effectively?
    - Would a reviewer understand the progression?
    - Are commits atomic (one logical change each)?

    Write findings to `findings/commit-discipline.md`.
    Rate each: P0 (must fix), P1 (should fix), P2 (nice to fix).

    Say "I'm done" when finished.
  PROMPT
}

agent "reviewer-tests" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Review `diff.patch` for TEST QUALITY.

    Coverage numbers lie. A test that can't fail provides no value.

    Look for:
    - Weak assertions
      - Only checking != nil / !== null / is not None
      - Using .is_ok() without checking the value
      - assertTrue(true) or equivalent
    - Missing negative test cases
      - Happy path only, no error cases
      - No boundary testing
      - No invalid input testing
    - Tests that can't fail
      - Mocked so heavily the test is meaningless
      - Testing implementation details, not behavior
    - Flaky test indicators
      - Sleep/delay in tests
      - Time-dependent assertions
    - Missing edge case and error path coverage

    Questions to answer:
    - Do these tests actually verify behavior?
    - Would a bug in the implementation cause a test failure?
    - Are edge cases and error paths tested?

    Write findings to `findings/test-quality.md`.
    Rate each: P0 (must fix), P1 (should fix), P2 (nice to fix).

    Say "I'm done" when finished.
  PROMPT
}

# ---------------------------------------------------------------------------
# Synthesis
# ---------------------------------------------------------------------------

agent "reviewer-synthesis" {
  run      = "claude --dangerously-skip-permissions --disallowed-tools ExitPlanMode,AskUserQuestion,EnterPlanMode"
  on_idle  = { action = "done" }
  on_dead  = { action = "done" }

  prompt = <<-PROMPT
    Synthesize review findings from `findings/` into a unified review.

    Read all findings files in the `findings/` directory.

    Write `findings/synthesis.md` with:
    1. **Executive Summary** — overall assessment (approve/request changes/block)
    2. **Critical Issues** — P0 from all legs, deduplicated
    3. **Major Issues** — P1, grouped by theme
    4. **Minor Issues** — P2, briefly listed
    5. **Wiring Gaps** — dependencies added but not used
    6. **Commit Quality** — notes on commit discipline
    7. **Test Quality** — assessment of test meaningfulness
    8. **Positive Notes** — what was done well
    9. **Recommendations** — actionable next steps

    Deduplicate issues found by multiple legs. Note which legs found them.
    Prioritize by impact and effort. Be actionable.

    Record each critical/major finding as a bead:
    ```bash
    bd create -t task --title "<finding>" --labels "review,severity:<level>"
    ```

    Say "I'm done" when finished.
  PROMPT
}
