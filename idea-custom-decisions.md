# Idea: Custom Decision Types via Runbooks

## Problem

Goblintown's decision system solves three things oddjobs doesn't:

1. **Proactive decisions** — agents surface choices to humans even when they could keep going (ambiguity, tradeoffs, checkpoints), not just when stuck/dead/errored
2. **Structured context requirements** — a "stuck" decision must say what was tried; a "tradeoff" must include a recommendation; prevents lazy "what should I do?" decisions
3. **Per-turn enforcement** — agents must check in with humans regularly, not run indefinitely

Goblintown hardcodes 8 decision types (ambiguity, checkpoint, confirmation, exception, prioritization, quality, stuck, tradeoff) with shell-script validators per type. The type is embedded as `_type` in the context JSON and validated at creation time. There's no agent-facing guidance on which type to use — agents only discover types by getting validation errors or being told by a predecessor decision.

Oddjobs handles the reactive side well (idle/dead/error/prompt escalation) but has no mechanism for proactive human check-ins during work that's going fine.

## Design Principles

- Policy lives in the runbook, not the binary
- Compose from existing primitives where possible
- Decision taxonomy is per-project, not per-tool

## Proposed Primitives

### 1. `on_signal` handler

Today `agent:signal` always completes the step. There's no way to inspect or react to what the agent signals before accepting it.

```hcl
agent "worker" {
  on_signal = { action = "gate", run = "check-decision-offered ${agent.id}" }
}
```

When the agent says it's done, the gate command runs. If it fails (non-zero), the agent gets nudged back. The gate script can check anything — was a `bd decision` created, do tests pass, does a PR exist.

This replaces goblintown's turn-check stop hook without building decision enforcement into the tool. The runbook decides what "done" means.

Valid actions: `done` (default/current behavior), `gate`, `escalate`, `fail`.

### 2. `prime.signal` source

Today prime supports `startup`, `resume`, `clear`, `compact`. Adding `signal` injects context when an agent signals completion but gets gated back:

```hcl
agent "worker" {
  prime = {
    startup = "wok show ${var.issue}"
    signal  = <<-SHELL
      echo "## Before ending your session"
      echo ""
      echo "You must offer a decision point. Use one of:"
      echo "- checkpoint: summarize progress + next steps"
      echo "- tradeoff: present options with a recommendation"
      echo "- stuck: describe blocker + what you tried"
      echo ""
      echo "Example:"
      echo '  bd decision create --prompt "..." --options '"'"'[{"id":"a","label":"..."},{"id":"b","label":"..."}]'"'"
    SHELL
  }
  on_signal = { action = "gate", run = "bd decision list --pending --json | jq -e 'length > 0'" }
}
```

Agent signals done → gate checks for a pending decision → fails → agent resumes with the `signal` prime telling it what to do.

### 3. Decision step type

A step that blocks the pipeline until a human decides. This is the main new primitive.

```hcl
job "feature" {
  step "plan" {
    run = { agent = "planner" }
  }
  step "approve" {
    decision {
      prompt = "Review the plan and choose how to proceed"
      option "build"  { description = "Plan looks good, build it" }
      option "revise" { description = "Send back for revision" }
      option "cancel" { description = "Scrap this feature" }
    }
  }
  step "build" {
    run = { agent = "builder" }
    depends_on = ["approve"]
  }
}
```

Emits `decision:created`, step enters `Waiting`, advances on `decision:resolved`. Context can come from a shell command:

```hcl
step "approve" {
  decision {
    prompt   = "Review implementation"
    context  = "git diff main...HEAD --stat"  # shell command, stdout becomes context
    option "merge" { description = "Ship it" recommended = true }
    option "fix"   { description = "Needs changes" }
  }
}
```

This replaces formula-driven decisions from goblintown (`[steps.decision]` in TOML) with the same concept expressed in HCL runbooks.

### 4. Decision templates (optional, convenience)

Reusable decision shapes that encode required context and default options:

```hcl
decision_template "checkpoint" {
  prompt_hint      = "Summarize progress and next steps"
  required_context = ["progress", "next_steps"]
  option "continue" { description = "Keep going" recommended = true }
  option "adjust"   { description = "Change direction" }
  option "stop"     { description = "Stop work" }
}

decision_template "tradeoff" {
  prompt_hint      = "Present competing options with a recommendation"
  required_context = ["options", "recommendation"]
}

decision_template "stuck" {
  prompt_hint      = "Describe what's blocking you and what you've tried"
  required_context = ["blocker", "tried"]
  option "unblock" { description = "Here's how to proceed" }
  option "reassign" { description = "Give to someone else" }
  option "cancel"  { description = "Drop this work" }
}
```

Used in steps:

```hcl
step "mid-check" {
  decision { template = "checkpoint" }
}
```

Or referenced by agents creating decisions via CLI — the executor validates context fields match the template. Different projects define different taxonomies.

This replaces goblintown's `validators/create-decision-type-*.sh` scripts with declarative validation in the runbook.

### 5. Agent prompt guidance (no new feature needed)

The existing `prompt` field handles this. Runbooks that want proactive decisions include guidance:

```hcl
agent "worker" {
  prompt = <<-PROMPT
    ${file(".oj/prompts/worker.md")}

    ## Decision Points
    When you encounter these situations, create a decision:
    - **Stuck**: bd decision create --prompt "..." --context '{"blocker":"...","tried":[...]}'
    - **Tradeoff**: Present options with your recommendation
    - **Checkpoint**: At natural stopping points, summarize progress
  PROMPT
}
```

This is a content problem, not a feature problem. The guidance lives in the runbook or in referenced prompt files, configurable per-project.

## Implementation Priority

1. **`on_signal` handler** — small extension, high value, enables turn enforcement
2. **Decision step type** — main new primitive, enables proactive pipeline gates
3. **`prime.signal` source** — small extension, completes the on_signal feedback loop
4. **Decision templates** — convenience layer, can come later
5. **Prompt guidance** — zero implementation work, just writing good prompts

## Beads Integration

All of this works with or without beads. With beads:
- Decision steps can optionally create `bd decision` beads for persistence/sync
- Gate scripts can check `bd decision list` for pending decisions
- Decisions get the full beads audit trail and JSONL export

Without beads:
- Decision steps use oddjobs' built-in decision storage (already exists)
- Gate scripts check whatever they want (test results, file existence, etc.)
- Decisions live in the oj WAL only

## Comparison

| Concern | Goblintown | Oddjobs (proposed) |
|---------|------------|-------------------|
| Turn enforcement | Built-in stop hook + turn marker files | `on_signal` + gate command (runbook-defined) |
| Decision types | 8 hardcoded types | Decision templates (runbook-defined) |
| Type validation | Shell scripts in `validators/` | `required_context` on templates |
| Agent guidance | Missing (no prompting for types) | `prompt` field in agent definition |
| Pipeline gates | Formula `[steps.decision]` in TOML | `decision {}` block in HCL steps |
| Context injection on gate failure | `gt decision remind --inject` | `prime.signal` source |
| Where policy lives | In the binary + shell scripts | In the runbook |
