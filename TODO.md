# TODO

Backlog (roughly priority-ordered):

Core pipeline
  1. Worker poll interval: optional poll_interval on worker blocks for periodic checks
  2. Worker concurrency > 1 (parallel/pool worker)
  3. Merge pipeline: handle non-fast-forward push (rebase-before-push or retry)

Reliability
  4. Cron: watchdog and janitor runbooks for stuck agents / stale resources
  5. Default error handling for agent errors (rate limit → retry, no internet → retry,
     out of credits → escalate, unauthorized → escalate). See design notes below.

Human-in-the-loop
  6. Status: quick status command showing active/escalated pipelines, queued merges,
     active agents, enabled crons
  7. Human In The Loop: CLI-first commands for handling escalation
  8. Terminal: Dedicated fullscreen TUI for "watching" status and interactive inbox
  9. Escalate on agent plan/question prompts instead of firing on_idle gate blindly
      - Detect plan-mode approval prompts and AskUserQuestion in agent log watcher
      - Emit distinct event (agent:prompt vs agent:idle) so gate doesn't fire
      - Surface the prompt to human via escalation (oj pipeline show / inbox)

Agent hooks
  10. Consider checking both native hooks and agent log for on_idle/on_error detection

Notifications
  11. Agent notify config: emit on_start, on_done, on_fail for agent lifecycle events
      - Pipeline notify is done; agent notify is parsed but not emitted in monitor.rs

CLI polish
  12. `oj daemon restart` (stop + start convenience command)
  13. CLI color output: detect tty, respect COLOR/NO_COLOR env vars
      - Copy color conventions from ../quench and ../wok
      - Colorize show, list, --help views and other human-facing output
      - Ask human for preferences on color scheme before implementing
  14. Add `oj agent list`

----

Key files:
  .oj/runbooks/bugfix.hcl         — fix worker pipeline
  .oj/runbooks/build.hcl          — feature build pipeline
  .oj/runbooks/merge.hcl          — local merge queue
  crates/runbook/src/find.rs      — runbook discovery (recursive scanner)
  crates/runbook/src/queue.rs     — QueueDef, QueueType
  crates/engine/src/spawn.rs      — agent spawn, prompt interpolation
  crates/engine/src/runtime/handlers/worker.rs — worker lifecycle
  crates/daemon/src/listener/     — request handlers (workers, queues, commands)
  crates/daemon/src/lifecycle.rs   — event processing, WAL persistence, state materialization

----

## Design Notes

### Agent Hooks

1. **Consider checking both hooks and agent log for on_idle/on_error.**
   Currently on_idle and on_error are detected from the agent session log by
   oj's monitor. Agent tools (Claude, Gemini) also have native hook events
   (Stop, Notification, etc.) that fire at the tool level before oj ever sees
   the state change. We should consider whether on_idle/on_error detection
   should use native hooks in addition to (or instead of) log-based monitoring.

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

Landed:
  - fix(engine): resume from terminal failure resets to failed step
  - fix(engine): shell-safe interpolation (interpolate_shell) for shell commands
  - fix(engine): agent idle/nudge/gate logging in pipeline logs and daemon traces
  - fix(runbooks): switch build/bugfix agents to nudge on_idle (not gate)
  - docs: agent entity description, notify block syntax in runbook CLAUDE.md
  - fix(engine): agent:signal complete overrides gate escalation (race condition fix)
  - fix(specs): resilient binary resolution for shared target dirs
  - fix(daemon): restore WorkerStop handler lost in merge
  - fix(cli): resolve worktree paths to main project root (merge dispatch fix)
  - fix(runbooks): use .cargo/config.toml instead of target symlink
  - fix(engine): kill tmux sessions on pipeline step transitions and failure
  - fix(engine): add tracing to persisted queue poll and worker wake flow
  - fix(runbooks): worktree init, feature branches with nonce, JSON queue push syntax
  - fix(runbooks): git -C $REPO push for worktree submit, wok done from invoke.dir
  - fix(cli): oj queue push uses positional queue name + JSON
  - fix(runbook): scanner recurses into subdirectories
  - fix(engine): reload runbook from disk on each worker poll (stale runbook fix)
  - fix(engine): enforce worker concurrency after daemon restart
  - fix(daemon): materialize WAL events so queries see runtime state
  - feat(worker-stop): oj worker stop command
  - feat(queue-items): oj queue items <queue> command
  - feat(resume-var): rename --input to --var on pipeline resume
  - feat(notify): pipeline notify blocks with on_start/on_done/on_fail (via notify-rust)
  - feat(runbook): add notify blocks to bugfix, build, and merge runbooks
  - feat(runbook): pipeline locals {} block for DRY variable definitions
  - feat(worker): make worker start idempotent, remove separate wake command
  - feat(queue): add --var key=value syntax to oj queue push
  - fix(runbook): surface skipped runbook errors when command not found
  - fix(engine): drop stale agent watcher events after pipeline advances
  - fix(submit): use --var syntax in runbook submit steps to avoid shell escaping issues

Observations:
  - Full fix loop works: oj run fix → agent fixes → submit → merge worker dispatches
  - Merge pipeline: init → merge → check → push (push fails on non-fast-forward if
    main has moved; merge runbook needs rebase-before-push or retry logic)
  - .cargo/config.toml approach for shared build cache works; no more symlink conflicts
  - Agents entering plan mode trigger on_idle immediately due to watcher initial state
    check bypassing idle timeout. Switched to nudge as workaround; proper fix is #9.
  - Implement agent exits after minimal work when on_idle gate fires during first
    thinking pause — gate passes because unused struct additions don't fail make check
  - Shared target dir causes stale test binaries when worktrees compile into it;
    fixed with runtime fallback in binary_path() but concurrent builds can still
    overwrite each other's binaries mid-test

Workflow patterns that work:
  - oj run fix → agent fixes → submit pushes branch + queue push → merge worker
    dispatches merge pipeline. Full loop works end-to-end now.
  - oj run build → plan agent → implement agent → submit → merge. Works end-to-end.
  - Submit steps use --var syntax to avoid shell escaping issues with titles
  - Pipeline locals {} reduce boilerplate (repo, branch, title defined once)
  - Cherry-pick from worktree branch when submit step fails (shell escaping, etc.)
  - Always make install + daemon restart after cherry-picking.
  - Clean up worktrees: oj workspace drop <id>

Known submit step issue:
  - Submit step shell interpolation of ${var.instructions} (long text with special chars)
    breaks commit messages and queue push. Locals + --var mitigate but don't fully solve;
    the full instructions string still gets interpolated into the commit -m argument.
