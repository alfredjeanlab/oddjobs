# TODO

In progress (agents working):
  - feat(cli): oj status command (dashboard overview)
  - feat(engine): step on_fail attempts support
  - chore(cli): sort list outputs by last updated
  - chore(cli): improve oj agent list columns (ID, NAME, PROJECT, shorter IDs)
  - chore(cli): add oj agent prune command

Recently landed:
  - fix(cli): inconsistent shell exit error messages
  - chore(cli): oj pipeline prune command
  - chore(cli): oj pipeline prune default 24h → 12h
  - fix(runbooks): fetch and rebase before push in merge pipeline
  - chore(runbooks): increase fix and chore worker concurrency to 3
  - chore(runbooks): rename agents to bugs and chores
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

Reliability
  2. Cron: watchdog and janitor runbooks for stuck agents / stale resources
  3. Default error handling for agent errors (rate limit → retry, no internet → retry,
     out of credits → escalate, unauthorized → escalate). See design notes below.

Human-in-the-loop
  4. Human In The Loop: CLI-first commands for handling escalation
  5. Terminal: Dedicated fullscreen TUI for "watching" status and interactive inbox

CLI polish
  6. CLI color output: detect tty, respect COLOR/NO_COLOR env vars
      - Copy color conventions from ../quench and ../wok
      - Colorize show, list, --help views and other human-facing output
      - Ask human for preferences on color scheme before implementing

Multi-project
  7. Shared queues: allow cross-project queue push (e.g. --project flag routing)
  8. Remote daemon: coordinate jobs across multiple machines

----

Key files:
  .oj/runbooks/bug.hcl            — fix worker pipeline
  .oj/runbooks/build.hcl          — feature build pipeline
  .oj/runbooks/chore.hcl          — chore worker pipeline
  .oj/runbooks/merge.hcl          — local merge queue
  crates/runbook/src/find.rs      — runbook discovery (recursive scanner)
  crates/runbook/src/queue.rs     — QueueDef, QueueType, RetryConfig
  crates/runbook/src/template.rs  — interpolation (shell escaping, trusted prefixes)
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
  - End-to-end pipelines: build, fix, chore all flow through submit → merge queue
  - Per-project namespace isolation (OJ_NAMESPACE) for multi-project use
  - Desktop notifications on pipeline/agent lifecycle (on_start/on_done/on_fail)
  - Notification hooks for instant agent idle/prompt detection (no polling)
  - Dead letter queue with retry semantics and configurable cooldown
  - Worker concurrency > 1 for parallel dispatch
  - Pipeline locals {}, name templates, on_cancel step, --var syntax throughout
  - WAL materialization for consistent query state across daemon restarts
  - Trusted-prefix interpolation: $(cmd) in locals works for shell steps
  - Ctrl+C handling in oj pipeline wait
  - Workspace drop resilience for orphaned worktrees
  - Auto-resume workers on daemon restart
  - Deduplicated workspace nonces, suppress empty Error: on silent exits

Patterns that work:
  - oj run {build,fix,chore} → agent → submit → merge queue. Full loop end-to-end.
  - Parallel builds: kick off multiple oj run build, monitor with
    oj pipeline wait <id1> <id2> <id3> for streaming progress.
  - Multi-project: run pipelines across oddjobs, wok, quench simultaneously.
  - Merge queue handles conflicts automatically via resolve agent.
  - Cherry-pick from worktree branch when a step fails, then make install + daemon restart.
  - Manual merge fallback: git fetch + merge when merge queue is stuck.
  - Commit and push TODO/doc changes before kicking off agents — avoids merge conflicts
    when agent branches diverge from main.
