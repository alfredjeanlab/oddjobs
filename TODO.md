# TODO

In progress (agents working):
  - feat(cli): oj worker stop <name> — pause a running worker
  - feat(cli): oj pipeline prune --failed, prune cancelled regardless of age
  - fix(engine): DeleteWorkspace should call git worktree remove before rm -rf

Drafts:
  - draft(cli): inline commands — execute shell command.run locally, not via daemon

Recently landed:
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

Backlog (roughly priority-ordered):

Core pipeline
  1. Worker poll interval: optional poll_interval on worker blocks for periodic checks
  2. Inline command execution: shell command.run executes locally (see draft)

Crons and reliability
  2. Cron entrypoint: time-driven pipeline execution (third entrypoint alongside commands/workers)
      Use cases:
        - Janitor (30m): prune stale worktrees, orphan tmux sessions, old logs (shell only)
        - Reliability engineer (1h): review failed pipelines, dead letters, recurring errors → file bugs
        - Security auditor (6h): review recent commits for vulnerabilities → file bugs
      See: docs/future/RUNBOOKS.md, docs/future/runbooks/{janitor,reliability,security}.hcl
      CLI: docs/future/CLI.md (oj cron {list,start,stop,once})
  3. Default error handling for agent errors (rate limit → retry, no internet → retry,
     out of credits → escalate, unauthorized → escalate). See design notes below.

Human-in-the-loop
  5. Human In The Loop: CLI-first commands for handling escalation
  6. Terminal: Dedicated fullscreen TUI for "watching" status and interactive inbox

CLI polish
  7. CLI color output: detect tty, respect COLOR/NO_COLOR env vars
      - Copy color conventions from ../quench and ../wok
      - Colorize show, list, --help views and other human-facing output
      - Ask human for preferences on color scheme before implementing

Multi-project
  8. Shared queues: allow cross-project queue push (e.g. --project flag routing)
  9. Remote daemon: coordinate jobs across multiple machines

----

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

Issues discovered:
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
  - Submit step fails when local.title contains special chars (quotes, $, backticks).
    Root cause: locals launder untrusted var.* content into trusted namespace.
    Fixed by eager locals (above). Workaround: manually push + queue merge.
