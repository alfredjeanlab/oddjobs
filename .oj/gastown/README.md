# Gas Town / Goblin Town Reimplemented as Runbooks

A reimplementation of Gas Town's multi-agent orchestration system using oj
runbooks + beads as the state layer.

## Architecture

```
beads (bd)                       oj runbooks
─────────────                    ───────────
State layer                      Execution layer

issues, bugs, tasks              jobs, steps
agent beads (identity)           agents (Claude sessions)
molecules (step tracking)        step transitions
merge-request beads              merge queue + worker
message beads (mail)             queue polling
advice beads                     prime context injection
```

**Principle: beads is truth, oj is motion.** Every piece of state — work items,
agent identity, mail, merge requests, escalations — lives in beads (`bd`).
Oj provides the jobs, agents, queues, and workers that execute against
that state.

## Agent Taxonomy

Gas Town defines a hierarchy of roles. Each role has a scope (town or rig)
and a specific function:

```
Town scope (GT_SCOPE=town)
├── Mayor       Coordinator — dispatches convoys, high-level decisions
├── Deacon      Town-level orchestrator — patrols, health, convoy tracking
├── Boot        Ephemeral triage — fresh each tick, single decision, exits
└── Dog         Shutdown enforcement — pure state machine, no AI

Rig scope (GT_SCOPE=rig)
├── Witness     Per-rig health monitor — processes mail, detects stalled/zombie
├── Refinery    Merge queue — sequential rebase, test, push
└── Polecat     Ephemeral worker — executes one issue, exits
```

### Watchdog Chain

The system has a multi-tier health monitoring chain:

```
Daemon (oj daemon)
  │  periodic heartbeat
  │
  └─► Boot (boot-triage.hcl)
      │  Fresh each tick, zero context — restarts workers, retries queues
      │
      └─► Deacon (deacon-patrol.hcl)
          │  Town-level patrol: inbox, escalations, convoys, dispatch
          │
          └─► Witness (witness-patrol.hcl)
                Per-rig patrol: polecat health, nudge stalled, resume escalated
```

Boot exists because the daemon can't reason and the deacon can't observe
itself. The separation costs complexity but enables intelligent triage
without constant AI cost.

## Directory Layout

```
gastown/
├── README.md               This file
├── infra.hcl               Shared queues, workers
├── start.hcl               Town startup and shutdown
├── sling.hcl               Work dispatch (spawn polecat for an issue)
├── convoy.hcl              Batched work tracking
├── formulas/
│   ├── polecat-work.hcl    Canonical polecat workflow (bugfix worker)
│   ├── witness-patrol.hcl  Per-rig health monitoring
│   ├── deacon-patrol.hcl   Town-level orchestration patrol
│   ├── refinery-patrol.hcl Merge queue processing
│   ├── boot-triage.hcl     Ephemeral watchdog triage
│   ├── shutdown-dance.hcl  Health check enforcement (Dogs)
│   └── code-review.hcl     Multi-leg code review
```

### File Inventory

| File | Commands | Jobs | Agents | Queues | Workers |
|------|----------|-----------|--------|--------|---------|
| infra.hcl | — | — | — | 5 | 2 |
| start.hcl | gt-start, gt-status, gt-stop | — | — | — | — |
| sling.hcl | gt-sling | sling | polecat-worker | — | — |
| convoy.hcl | gt-convoy, gt-convoy-status, gt-convoy-dispatch | convoy-dispatch | convoy-dispatcher | — | — |
| polecat-work.hcl | — | polecat-work | polecat | — | — |
| witness-patrol.hcl | gt-witness-patrol | witness-patrol | witness-agent | — | — |
| deacon-patrol.hcl | gt-deacon-patrol | deacon-patrol | deacon-agent | — | — |
| refinery-patrol.hcl | — | refinery-patrol | refinery-agent | — | — |
| boot-triage.hcl | gt-triage | boot-triage | boot-agent | — | — |
| shutdown-dance.hcl | gt-shutdown-dance | shutdown-dance | — | — | — |
| code-review.hcl | gt-review | code-review | 12 | — | — |

## Commands Reference

```bash
# Startup / Lifecycle
oj run gt-start [--rig <rig>]           # Initialize town, start workers
oj run gt-status                        # Check system health
oj run gt-stop                          # Graceful shutdown

# Work Dispatch
oj run gt-sling <issue> <instructions> [--base <branch>] [--rig <rig>]

# Batch Work
oj run gt-convoy <name> <issues...>     # Create convoy tracking multiple issues
oj run gt-convoy-status [--id <id>]     # Show convoy or list all active
oj run gt-convoy-dispatch <id> [--base <branch>] [--rig <rig>]

# Patrols
oj run gt-triage                        # Boot: ephemeral watchdog triage
oj run gt-deacon-patrol                 # Deacon: town-level patrol
oj run gt-witness-patrol [--rig <rig>]  # Witness: per-rig health monitor

# Enforcement
oj run gt-shutdown-dance <target> <reason>

# Code Review
oj run gt-review <branch> [--base <base>]
```

## Conventions

### Environment Variables

Every agent gets identity via the same environment variables Gas Town uses:

| Variable | Purpose | Example |
|----------|---------|---------|
| `GT_ROLE` | Agent role | `polecat`, `witness`, `refinery`, `deacon`, `mayor`, `boot` |
| `GT_RIG` | Rig name (rig-scoped roles) | `default`, `myproject` |
| `GT_POLECAT` | Polecat name (polecat role only) | `toast`, `nux` |
| `GT_SCOPE` | Scope level | `town` or `rig` |
| `BD_ACTOR` | Bead identity for audit | `default/polecats/toast`, `default/witness` |
| `GIT_AUTHOR_NAME` | Git author (polecats use just the name) | `toast` |

### Beads as State

All state mutations go through `bd`:

| State           | Beads command                              |
|-----------------|--------------------------------------------|
| Create work     | `bd create -t task --title "..."`          |
| Track agent     | `bd create -t agent --id <agent-id>`       |
| Send mail       | `bd create -t message --labels "to:witness,msg-type:polecat-done"` |
| Submit MR       | `bd create -t merge-request ...`           |
| Find next step  | `bd ready --parent=<molecule-id> --json`   |
| Close step      | `bd close <step-id>`                       |
| Add advice      | `bd advice add "..." --role polecat`       |
| Query advice    | `bd advice list --for=<agent-id>`          |
| Track convoy    | `bd dep add <convoy> <issue> --type=tracks`|
| File escalation | `bd create -t task --labels "escalation,severity:medium"` |

### Work Discovery

Agents don't hardcode what to do. They discover work:

1. `bd show <hook-bead>` — what's on my hook?
2. `bd ready --parent=<mol-id>` — what step is next?
3. `bd show <step-id>` — what does this step require?

### Mail Protocol

Inter-agent messages use the `message` bead type (a custom type configured at
startup) with label-based routing:

```bash
# Send
bd create -t message \
  --title "POLECAT_DONE toast" \
  --description "Exit: MERGED\nIssue: gt-abc\nBranch: polecat/toast-xyz" \
  --labels "from:default/polecats/toast,to:witness,msg-type:polecat-done"

# Receive (check inbox)
bd list -t message --label "to:witness" --status open --json
```

Message types: `polecat-done`, `merged`, `dog-done`, `lifecycle`.

### Molecule Step Format

Molecules are beads with child step beads:

```
parent: <molecule-id>
title: "Step N: <title>"
description: "Instructions for this step..."
```

Steps are discovered via `bd ready --parent=<mol-id>` and closed via
`bd close <step-id>`.

## Flows

### Single Issue: sling → polecat → refinery → merge

```
Human: oj run gt-sling auth-fix "Fix the auth bug"
  │
  ├─ 1. Create task bead (or use existing bead ID)
  ├─ 2. Create molecule steps as child beads
  ├─ 3. Spawn polecat job in ephemeral worktree
  ├─ 4. Polecat discovers steps via bd ready, executes each
  ├─ 5. Polecat commits, pushes branch, creates MR bead, sends POLECAT_DONE
  │
  ├─ 6. Refinery worker picks up MR from merge-requests queue
  ├─ 7. Rebase feature onto main, run tests
  │     (conflicts/failures → refinery-agent resolves or escalates)
  ├─ 8. Push to main, send MERGED mail, close MR, delete branch
  │
  └─ 9. Witness processes MERGED mail, cleans up polecat workspace
```

### Batch Work: convoy → sling (x N) → track completion

```
Human: oj run gt-convoy "Auth overhaul" issue-1 issue-2 issue-3
  │
  ├─ 1. Create convoy bead, link issues via bd dep add --type=tracks
  │
  ├─ Human: oj run gt-convoy-dispatch <convoy-id>
  │  └─ Dispatcher reads tracked issues, calls oj run gt-sling for each
  │
  ├─ (Each issue follows the sling → polecat → refinery flow above)
  │
  └─ Deacon patrol auto-closes convoy when all tracked issues are closed
```

### Health Monitoring: boot → deacon → witness

```
Daemon heartbeat (periodic):
  │
  └─ oj run gt-triage
     │
     ├─ Observe: oj status, workers, queues, escalations
     ├─ Act: restart stopped workers, retry dead queue items,
     │       resume/cancel escalated jobs
     └─ Exit (ephemeral — zero accumulated context)

Deacon patrol (periodic):
  │
  ├─ Process inbox (to:deacon messages)
  ├─ Auto-close completed convoys
  ├─ Handle escalation beads
  ├─ Dispatch undispatched ready work via gt-sling
  └─ Restart stopped workers, retry dead queue items

Witness patrol (periodic, per-rig):
  │
  ├─ Process inbox (POLECAT_DONE, MERGED)
  ├─ Health scan: nudge stalled agents, resume escalated jobs
  ├─ Restart stopped workers
  └─ Report findings
```

### Shutdown Dance: warrant → interrogate → pardon/execute

```
oj run gt-shutdown-dance <target> "unresponsive"
  │
  ├─ Record warrant bead
  ├─ Sleep 60s  → check bd + oj status → alive? PARDON
  ├─ Sleep 120s → check bd + oj status → alive? PARDON
  ├─ Sleep 240s → check bd + oj status → alive? PARDON
  └─ All 3 failed → EXECUTE (close warrant, create escalation, notify deacon)
```

No AI agents involved — pure shell state machine with escalating timeouts,
matching Gas Town's design where dogs are lightweight goroutines.

## Design Principles

1. **Beads is truth, oj is motion** — durable state in beads, execution in oj
2. **Propulsion Principle** — work on hook = execute immediately; no idle state
3. **Agent-driven decisions (ZFC #5)** — Claude decides on conflicts, not code
4. **Idle Town Principle** — healthy system is silent; no log spam
5. **Self-cleaning** — ephemeral agents exit after task completion
6. **Rebase-as-work** — conflicts spawn fresh polecats, never "sent back"
7. **Sequential processing** — one merge at a time prevents cascading conflicts
8. **Dogs are state machines** — shutdown dances are pure shell, no AI cost
9. **Mail protocol** — inter-agent communication via message beads with label routing
10. **Fresh context** — boot runs ephemeral each tick; no accumulated context debt

## Limitations

- **No parallel steps** — Gas Town's code review runs 10 legs in parallel;
  oj runs all 10 sequentially. Could use workers with `concurrency > 1` but
  adds orchestration complexity (queue + worker + polling for completion).
- **Commands can't run agents directly** — only jobs can reference agents
  via `run = { agent = "..." }`. Commands that need an agent require a
  single-step job wrapper (e.g. `convoy-dispatch`).
