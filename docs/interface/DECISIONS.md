# Decisions

Human-in-the-loop decisions let jobs and agents pause for human input when they encounter situations they can't resolve autonomously.

## Overview

A decision is created when an agent escalates — via `on_idle`, `on_dead`, `on_error`, `on_prompt`, or a failed `gate` command. The owning job (or crew) enters a waiting state until the decision is resolved. Each decision carries a context message explaining what happened and a set of numbered options appropriate to the escalation source.

## CLI

### oj decision list

List pending (unresolved) decisions.

```bash
oj decision list                     # List pending decisions
oj decision list --project <name>    # Filter by project namespace
oj decision list -o json             # JSON output
```

### oj decision show

Show full details of a decision including context and options.

```bash
oj decision show <id>                # Show decision details
oj decision show <id> -o json        # JSON output
```

The `<id>` argument supports prefix matching — a unique prefix of the decision ID is sufficient.

### oj decision resolve

Resolve a single decision directly.

```bash
oj decision resolve <id> 1           # Pick option #1
oj decision resolve <id> -m "msg"    # Resolve with freeform message
oj decision resolve <id> 2 -m "msg"  # Pick option with additional message
```

Resolving a decision triggers the action mapped to the chosen option (see [Option Mapping](#option-mapping) below) and advances or terminates the owning job.

### oj decision review

Interactively walk through all pending decisions.

```bash
oj decision review                   # Review all pending decisions
oj decision review --project <name>  # Filter by project namespace
```

For each unresolved decision, `review` displays the full context and options, then prompts for input:
- `1`–`N` — pick a numbered option
- `s` — skip this decision
- `q` — quit review

After picking an option, you can optionally provide a freeform message. At the end, a summary shows how many decisions were resolved and skipped.

## Decision Sources

Decisions are created by different escalation triggers, each with its own default options:

| Source | Trigger | Default Options |
|--------|---------|-----------------|
| `idle` | Agent idle (stop hook fired) | Nudge *(rec)*, Done, Cancel, Dismiss |
| `dead` | Agent process exited unexpectedly | Retry *(rec)*, Skip, Cancel, Dismiss |
| `error` | Agent API/runtime error | Retry *(rec)*, Skip, Cancel, Dismiss |
| `gate` | Gate command exited non-zero | Retry *(rec)*, Skip, Cancel |
| `approval` | Agent showing a permission prompt | Approve, Deny, Cancel, Dismiss |
| `question` | Agent called `AskUserQuestion` tool | User-provided options + Other, Cancel, Dismiss |
| `plan` | Agent called `ExitPlanMode` | Accept (clear) *(rec)*, Accept (auto), Accept (manual), Revise, Cancel |

*(rec)* = marked as recommended.

## Option Mapping

When a decision is resolved, the chosen option maps to a concrete action on the owning job or agent:

### Idle decisions

| Option | Action |
|--------|--------|
| 1 — Nudge | Resume job, send nudge message to agent |
| 2 — Done | Complete the current step, advance job |
| 3 — Cancel | Cancel the job |
| 4 — Dismiss | No action (decision acknowledged, job stays waiting) |

### Dead / Error decisions

| Option | Action |
|--------|--------|
| 1 — Retry | Resume job, restart agent |
| 2 — Skip | Complete the current step, advance job |
| 3 — Cancel | Cancel the job |
| 4 — Dismiss | No action (decision acknowledged) |

### Gate decisions

| Option | Action |
|--------|--------|
| 1 — Retry | Resume job, restart agent |
| 2 — Skip | Complete the current step, advance job |
| 3 — Cancel | Cancel the job |

### Approval decisions

| Option | Action |
|--------|--------|
| 1 — Approve | Send `y` to agent session |
| 2 — Deny | Send `n` to agent session |
| 3 — Cancel | Cancel the job |
| 4 — Dismiss | No action (decision acknowledged) |

### Question decisions

| Option | Action |
|--------|--------|
| 1–N | Send chosen option number to agent session |
| N+1 — Other | Prompt for custom text response |
| N+2 — Cancel | Cancel the job |
| N+3 — Dismiss | No action (decision acknowledged) |

### Plan decisions

| Option | Action |
|--------|--------|
| 1 — Accept (clear context) | Approve plan, clear context window |
| 2 — Accept (auto edits) | Approve plan, auto-apply edits |
| 3 — Accept (manual edits) | Approve plan, manual edit mode |
| 4 — Revise | Send revision message to agent |
| 5 — Cancel | Cancel the job |

A freeform message (`-m`) on any decision type is forwarded to the agent as a resume message.

## Events

Two events track the decision lifecycle:

| Type tag | Variant | Fields |
|----------|---------|--------|
| `decision:created` | DecisionCreated | `id`, `job_id`, `agent_id?`, `owner`, `source`, `context`, `options[]`, `created_at_ms`, `project` |
| `decision:resolved` | DecisionResolved | `id`, `chosen?`, `message?`, `resolved_at_ms`, `project` |

`decision:created` sets the owning job's step to `Waiting(decision_id)`. `decision:resolved` updates the decision record and emits the mapped action event (e.g. `job:resume`, `job:cancel`, `step:completed`).

## Lifecycle

```text
Escalation trigger
  → DecisionCreated event
  → Job enters Waiting state
  → Decision appears in `oj decision list`

Human resolves decision
  → DecisionResolved event
  → Mapped action event emitted (JobResume, JobCancel, StepCompleted, SessionInput)
  → Job advances or terminates
```

### Cleanup

- When a job reaches a terminal state (done, cancelled, failed): **unresolved** decisions for that job are removed; **resolved** decisions are preserved as an audit trail.
- When a job is deleted: **all** decisions (resolved and unresolved) are removed.
