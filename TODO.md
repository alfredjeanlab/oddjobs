# TODO

## Backlog

Decisions (human-in-the-loop) — see plans/decision-roadmap.txt
  1. Decision debug bundle: crumb + debug.txt per decision (phase 2)
  2. Escalation decisions: system-generated options for gate/idle/dead/error (phase 3)
  3. AskUserQuestion decisions: capture payload, answer flows back to agent (phase 4)
  4. Plan approval decisions: capture ExitPlanMode payload, approve/reject (phase 5)
  5. Decision polish: status count, auto-expire on cancel, --resolved history (phase 6)
  6. Terminal: Dedicated fullscreen TUI for decisions and status

Reliability
  7. Queue crumb files: {logs_dir}/queue/{namespace}/{queue_name}.crumb.json — full queue snapshot on every state change
  8. Namespace log paths: queue/worker/cron logs collide across projects, need {namespace}/ prefix
  9. Default error handling for agent errors (rate limit → retry, no internet → retry,
     out of credits → escalate, unauthorized → escalate). See design notes below.

CLI polish
  10. CLI color output: extend colors beyond help to list, show, status views
      - Colors landed for clap help; next step is colorizing data output
  11. "Did you mean?" suggestions for typos and cross-project lookups
      - Fuzzy-match resource names (queues, workers, crons) on not-found errors
      - Suggest --project when resource exists in another namespace
      - Build dispatched (pipeline e9cb54ef)

Multi-project
  12. Remote daemon: coordinate jobs across multiple machines

----

## In Flight
  - fix(engine): queue items stuck in Active status after worker restart (pipeline bb7f78ff)
  - chore(cli): `oj queue drop` should accept ID prefixes (pipeline d12e5c65)
  - feat(cli): "did you mean?" suggestions for resource name typos and cross-project lookups (pipeline e9cb54ef)

## Recently Completed
  - feat(engine): standalone agent runs — `oj run <name>` with `run = { agent = "..." }` spawns top-level agents
  - feat(engine): decision system phase 1 — data model, storage, events, CLI (oj decision list/show/resolve)
  - feat(cli): TIME column in `oj cron list` (next fire countdown for running, last fired age for stopped)
  - fix(cli): rename NAME → KIND column in `oj cron list`
  - chore(runbooks): prime merge resolver with git status, commit log, and diffstat
  - chore(runbooks): town start/stop manages project workers across all projects
  - fix(engine): `oj cron once` pipelines silently vanish (CommandRun vs create_and_start_pipeline)
  - feat(runbooks): janitor cron — periodic prune of pipelines, agents, workers, workspaces
  - feat(runbooks): medic cron — triage failures, stale work, stuck merges → symptoms queue
  - feat(runbooks): heartbeat cron — temporary debug cron for verifying timer execution
  - feat(runbooks): town start command — launches crons
  - chore(docs): updated runbook guide with crons, prime, inbox queues, best practices
  - feat(cli): ANSI color support for clap help output (header/literal/context/muted)
  - fix(cli): `oj run` with no arguments exits 0 (not error)
  - fix(cli): `oj run errand -h` shows errand-specific help (not generic oj run help)
  - fix(cli): remove vestigial -a/--arg and --daemon flags from `oj run`
  - chore(cli): short IDs use 8 hex characters consistently (no hyphen suffix)
  - chore(cli): --project flag on cron start/stop/once commands
  - chore(cli): --project flag on worker list/prune commands
  - chore(cli): --project flag on pipeline prune and workspace prune commands
  - chore(cli): dynamic PROJECT column in `oj session list`
  - feat(cli): `oj agent show <id>` — detail view for agents
  - feat(cli): `oj cron prune` — clean up stopped cron entries
  - feat(cli): `oj status --watch` — auto-refreshing status display
  - feat(cli): `oj peek/attach/logs/show <id>` — top-level convenience commands
  - feat(runbook): `poll` option on external queue blocks (30s, 5m, 8h durations)
  - feat(cli): `oj worker logs`, `oj cron logs`, `oj queue logs` with -f/--follow, --project
  - chore(cli): pipeline logs moved to {logs_dir}/pipeline/ subdir (consistent with agent/)
  - chore(cli): `oj daemon logs` uses -n/--limit/--no-limit (consistent with other logs)
  - feat(engine): source-aware prime commands for agents (per-source SessionStart hooks)
  - feat(cli): `oj pipeline prune --orphans` — prune orphaned pipelines via breadcrumb files
  - fix(cli): `oj queue list` shows all queues across all project namespaces
  - fix(cli): external queue push runs list command from project root
  - fix(cli): rename "(default)" namespace label to "(no project)" in display output
  - feat(cli): show total retry count across steps in `oj pipeline list`
  - feat(cli): `oj project list` — list project name and root directory
  - feat(daemon): surface orphaned pipelines in pipeline list/show/status
  - feat(cli): add --project flag to worker start/stop commands
  - feat(engine): cron entrypoint — time-driven pipeline execution (oj cron {list,start,stop,once})
  - chore(runbooks): replace shared target-dir with sccache across all projects
  - fix(runbooks): stage uncommitted resolver changes before rebase in merge push
  - fix(test): clear GIT_DIR/GIT_WORK_TREE so executor tests pass inside worktrees
  - feat(cli): inline command execution — shell command.run executes locally, not via daemon
  - fix(cli): use bash with pipefail for inline shell commands
  - feat(cli): oj worker prune — remove stopped workers from state
  - feat(cli): oj worker stop <name> — pause a running worker
  - feat(cli): oj pipeline prune --failed, prune cancelled regardless of age
  - fix(engine): DeleteWorkspace should call git worktree remove before rm -rf
  - chore(runbooks): add draft-rebase command and update CLAUDE.md
  - chore(runbooks): sync build.hcl paradigms across projects
  - chore(docs): modernize future cron runbooks, add architect.hcl
  - chore(docs): update cron design — use cases, start/stop/once naming
  - fix(runbooks): make branch deletion non-fatal in merge push step
  - fix(runbooks): use local branches in draft-rebase/refine worktrees
  - chore(runbooks): add worktree cleanup steps to all pipelines (on_done/on_cancel/on_fail)
  - chore(runbooks): merge push on_fail attempts = 2, resolver gate attempts = 2
  - feat(engine): eager locals — evaluate $() in locals at creation, remove trusted prefixes
  - feat(engine): PreToolUse hook for ExitPlanMode/AskUserQuestion → Prompting state
  - feat(runbook): validate name references and step reachability at parse time
  - fix(runbook): validate template references (var/args/local) at parse time
  - feat(engine): step on_fail attempts support
  - feat(cli): auto-generate --help for runbook commands (oj run <cmd> --help)
  - feat(engine): pipeline breadcrumb files for orphan detection
  - feat(cli): oj status command (dashboard overview)
  - feat(cli): refactor oj queue CLI (list queues, items subcommand)
  - fix(parser): handle multi-value CLI options (--disallowed-tools, --allowed-tools)
  - fix(cli): oj pipeline peek fallback when session gone
  - chore(runbooks): add oj run merge <branch> <title> convenience command
  - chore(runbooks): add draft.hcl — oj run draft, oj run drafts
  - chore(runbooks): update comment format for auto-help (description/examples)
  - chore(runbooks): fetch+rebase in merge push across all projects
  - chore(cli): sort list outputs by last updated
  - chore(cli): improve oj agent list columns (ID, NAME, PROJECT, shorter IDs)
  - chore(cli): add oj agent prune command
  - chore(cli): oj pipeline prune default 24h → 12h
  - chore(runbooks): on_cancel → close issue, on_fail → reopen issue
  - chore(runbooks): disallow ExitPlanMode/AskUserQuestion in fix/chore agents
  - chore(runbooks): increase fix and chore worker concurrency to 3
  - chore(runbooks): rename agents to bugs and chores
  - fix(cli): inconsistent shell exit error messages
  - chore(cli): oj pipeline prune command
  - fix(workspace): oj workspace drop for orphaned worktrees
  - fix(daemon): auto-resume workers on daemon restart
  - fix(cli): suppress empty 'Error: ' on silent exit codes
  - fix(cli): deduplicate workspace nonces (ws-foo-abc-abc → ws-foo-abc)
  - fix(engine): allow $(cmd) in pipeline locals (interpolate_shell_trusted)
  - fix(cli): oj pipeline wait Ctrl+C handling
  - chore(runbooks): use local.repo in locals across oddjobs, wok, quench
  - chore(runbooks): add pipeline name templates to wok and quench
  - chore(docs): update oddjobs docs for recent features

Key files:
  .oj/runbooks/bug.hcl            — fix worker pipeline
  .oj/runbooks/build.hcl          — feature build pipeline
  .oj/runbooks/chore.hcl          — chore worker pipeline
  .oj/runbooks/draft.hcl          — draft pipeline + drafts list/close
  .oj/runbooks/merge.hcl          — local merge queue
  crates/runbook/src/find.rs      — runbook discovery (recursive scanner)
  crates/runbook/src/help.rs      — auto-generated --help for runbook commands
  crates/runbook/src/queue.rs     — QueueDef, QueueType, RetryConfig
  crates/runbook/src/template.rs  — interpolation (shell escaping)
  crates/engine/src/spawn.rs      — agent spawn, prompt interpolation
  crates/engine/src/workspace.rs  — agent settings injection (Stop, Notification hooks)
  crates/engine/src/monitor.rs    — MonitorState, PromptType, action effects
  crates/engine/src/runtime/handlers/worker.rs — worker lifecycle, dead letter
  crates/daemon/src/listener/     — request handlers (workers, queues, commands)
  crates/daemon/src/lifecycle.rs   — event processing, WAL persistence, state materialization

----

## Design Notes

### Default Error Handling

Add sensible default `on_error` behavior for each `AgentError` variant so
agents recover automatically without requiring explicit runbook configuration.

Currently all four error types are categorized as `Failed` states and trigger
`on_error` (default: `"escalate"`). Most of these should retry automatically:

1. **`RateLimited`** — default should auto-retry with jittered backoff
   (e.g. `gate` with `run = "sleep $((60 + RANDOM % 30))"`,
   `attempts = "forever"`, `cooldown = "60s"`). Rate limits are transient
   and almost always resolve on their own.

2. **`NoInternet`** — default should auto-retry with jittered backoff
   (e.g. `gate` with `run = "sleep $((15 + RANDOM % 30))"`,
   `attempts = "forever"`, `cooldown = "30s"`). Network blips are transient;
   escalating immediately is too aggressive.

3. **`OutOfCredits`** — default should escalate. Requires human intervention.

4. **`Unauthorized`** — default should escalate (or fail). Invalid API key
   won't fix itself.

Open questions:
- Should the defaults be baked into the engine (hardcoded fallback when no
  `on_error` is configured), or expressed as default runbook values?
- What backoff strategy for rate limits? Fixed delay, exponential, or
  adaptive based on the retry-after header (if available in the log)?
- Should `NoInternet` have a max attempt count before escalating, or retry
  indefinitely on the assumption connectivity will return?

----

## Dogfooding Notes

Key features landed:
  - End-to-end pipelines: build, fix, chore, draft all flow through agents
  - Per-project namespace isolation (OJ_NAMESPACE) for multi-project use
  - Desktop notifications on pipeline/agent lifecycle (on_start/on_done/on_fail)
  - Notification hooks for instant agent idle/prompt detection (no polling)
  - Dead letter queue with retry semantics and configurable cooldown
  - Worker concurrency > 1 for parallel dispatch (tested with concurrency=3)
  - Pipeline locals {}, name templates, on_cancel step, --var syntax throughout
  - WAL materialization for consistent query state across daemon restarts
  - Eager locals: $() evaluated at creation, all shell interpolation escaped uniformly
  - Ctrl+C handling in oj pipeline wait
  - Workspace drop resilience for orphaned worktrees
  - Auto-resume workers on daemon restart
  - Deduplicated workspace nonces, suppress empty Error: on silent exits
  - Pipeline on_cancel/on_fail → wok issue lifecycle (close/reopen)
  - --disallowed-tools to prevent agents from using plan mode or asking questions
  - Auto-generated --help for runbook commands
  - Runbook validation: name references, step reachability, template references
  - Draft runbook: exploratory work pushed to draft/ branches, not merged
  - `oj run merge` convenience command across all projects
  - PreToolUse hook: detect plan/question tools before agent gets stuck
  - Step on_fail attempts: bounded retry for push→check loops
  - Worktree cleanup steps in all runbooks (build, fix, chore, draft, merge)
  - Inline command execution: shell commands run locally without daemon round-trip
  - Worker stop/prune: lifecycle management for workers
  - Draft-rebase command for rebasing exploratory branches
  - Cron entrypoint: time-driven pipeline execution with auto-resume and runbook hot-reload
  - sccache for worktree builds: eliminates shared target-dir cache poisoning
  - Orphan pipeline detection: breadcrumb files + surfacing in list/show/status
  - `oj project list`: visibility into registered projects and roots
  - --project flag on worker start/stop: explicit namespace control
  - Source-aware prime: per-source SessionStart hooks for agents
  - `oj pipeline prune --orphans`: clean up orphaned pipelines
  - Cross-namespace queue visibility: `oj queue list` shows all projects
  - External queue push fix: list command runs from project root
  - Retry count in pipeline list: visibility into step retry activity
  - ANSI color support for clap help (matching wok/quench color conventions)
  - Consistent --project flag across queue, worker, cron, pipeline prune, workspace prune
  - Top-level convenience commands: oj peek/attach/logs/show with auto entity resolution
  - oj agent show: detail view for individual agents
  - oj cron prune: lifecycle cleanup for stopped crons
  - oj status --watch: auto-refreshing dashboard
  - Queue poll interval: timer-based wake for external queues
  - Per-entity log commands: oj worker/cron/queue logs with instrumentation
  - Pipeline logs moved to pipeline/ subdir (consistent with agent/)
  - Consistent log flags: -n/--limit/--no-limit across all log commands
  - Short IDs: 8 hex chars everywhere (no hyphen suffix)
  - Cron runbooks: janitor (prune), medic (triage → symptoms queue), heartbeat (debug)
  - Town start command: `oj run start` launches crons and project workers
  - Runbook guide: documented crons, prime, inbox queues, step consolidation
  - Standalone agent runs: `oj run <name>` with `run = { agent = "..." }` as top-level WAL entity
  - Decision system phase 1: data model, WAL events, oj decision list/show/resolve
  - Cron list: TIME column (next fire / last fired), KIND column rename
  - Merge resolver prime: git status + commit log + diffstat injected at session start

Patterns that work:
  - oj run {build,fix,chore,draft} → agent → submit/push. Full loop end-to-end.
  - Parallel builds: kick off multiple oj run build, monitor with
    oj pipeline wait <id1> <id2> <id3> for streaming progress.
  - Multi-project: run pipelines across oddjobs, wok, quench simultaneously.
  - Merge queue handles conflicts automatically via resolve agent.
  - Merge queue: fetch+rebase before push handles non-fast-forward when main moves.
  - Cherry-pick from worktree branch when a step fails, then make install + daemon restart.
  - Manual merge fallback: git fetch + merge when merge queue is stuck.
  - Commit and push TODO/doc changes before kicking off agents — avoids merge conflicts
    when agent branches diverge from main.
  - oj run drafts to review exploratory work without polluting main.
  - Prime commands to pre-load agent context (oj status, queue list, etc.) — saves tool calls.
  - Persisted queues as inboxes: agents push findings, humans review in bulk later.
  - Combine independent shell commands in one step — don't over-split simple pipelines.

Issues discovered:
  - `oj cron once` pipelines silently vanish. Root cause: handle_cron_once emits CommandRun
    which looks up a `command` block, but cron runbooks have no command block. Fix: use
    create_and_start_pipeline directly (same path as cron timer fire). Build dispatched.
  - ExitPlanMode/AskUserQuestion tools block agents at TUI dialogs with no hook signal.
    Claude Code only fires idle_prompt after 60s, and the agent is mid-tool-call, not idle.
    Workaround: --disallowed-tools. Fixed: PreToolUse hook detects plan/question tools.
  - Pipeline state can be lost on daemon restart (WAL durability gap). Breadcrumb files
    now written for orphan detection.
  - Runbook parser treated multi-value options (--disallowed-tools A B) as positional args.
    Fixed: parser now handles multi-value CLI options correctly.
  - Shell escaping bug: var.* values flowing through locals bypass escaping.
    Fixed: eager locals evaluate $() at creation, all interpolation escaped uniformly.
  - Merge push→check loop: on_fail cycle with no max attempts causes infinite retries.
    Fixed: step on_fail attempts support landed.
  - Merge push fails when resolver leaves unstaged changes: rebase aborts on dirty worktree.
    Fixed: push step now stages and amends before rebasing.
  - Shared target-dir between worktrees causes cache poisoning (stale crate builds).
    Fixed: replaced with sccache — each worktree gets own target dir, artifacts cached globally.
  - Submit step fails when local.title contains special chars (quotes, $, backticks).
    Root cause: locals launder untrusted var.* content into trusted namespace.
    Fixed by eager locals (above). Workaround: manually push + queue merge.
  - Queue items stuck in Active status after worker/daemon restart. Root cause:
    PipelineAdvanced removes pipeline from active_pipeline_ids in materialized state,
    but if check_worker_pipeline_complete doesn't run (restart between WAL apply and
    engine handler), no QueueCompleted event is emitted. The item_pipeline_map is
    in-memory only and reconstruction misses completed pipelines. Fix dispatched.
  - `oj queue drop` requires full UUID while other queue commands accept prefixes.
    Chore dispatched.
