# CLI Reference

The `oj` command is a thin client that communicates with the `ojd` daemon. Most commands send events or queries over a Unix socket; the daemon owns the event loop and state.

See [Daemon Architecture](../arch/01-daemon.md) for details on the process split.

## Project Structure

```text
<project>/
└── .oj/
    ├── config.toml          # Project config (optional)
    └── runbooks/            # Runbook files (.hcl, .toml, or .json)
        ├── build.hcl        # oj run build ...
        ├── bugfix.hcl       # oj run fix ...
        └── ...
```

CLI finds the project root by walking up from cwd looking for `.oj/` directory.

## Daemon

### oj daemon

Manage the background daemon.

```bash
oj daemon start              # Start daemon (background)
oj daemon start --foreground # Start in foreground (debugging)
oj daemon stop               # Graceful shutdown (sessions preserved)
oj daemon stop --kill        # Stop and terminate all sessions
oj daemon status             # Health check
oj daemon logs               # View daemon logs
oj daemon logs --follow      # Stream logs (alias: -f)
oj daemon logs -n 100        # Show last N lines (default: 50)
oj daemon logs --no-limit    # Show all lines
```

The daemon auto-starts on first command if not already running.
Explicit `oj daemon start` is only needed for debugging or custom configurations.

## Entrypoints

### oj run

Execute commands defined in runbooks.

```bash
oj run <command> [args...]
oj run build auth "Add authentication"
oj run build auth "Add auth" -a priority=1
oj run build --runbook path/to/custom.hcl auth "Add auth"
```

Named arguments are passed with `-a`/`--arg key=value` and are available in the runbook as `var.<key>`.

## Resources

### oj pipeline

Manage running pipelines.

```bash
oj pipeline list                     # List all pipelines
oj pipeline list build               # Filter by runbook name
oj pipeline list --status running    # Filter by status
oj pipeline list -n 50               # Limit results (default: 20)
oj pipeline list --no-limit          # Show all results
oj pipeline show <id>                # Shows Project: field when namespace is set
oj pipeline resume <id>
oj pipeline resume <id> -m "message" --var key=value
oj pipeline cancel <id> [id...]
oj pipeline attach <id>              # Attach to active agent session
oj pipeline peek <id>                # View agent session output
oj pipeline logs <id>                # View pipeline logs
oj pipeline logs <id> --follow       # Stream logs (alias: -f)
oj pipeline logs <id> -n 100         # Limit lines (default: 50)
oj pipeline wait <id>                # Wait for pipeline completion
oj pipeline wait <id> --timeout 30m  # With timeout (human-readable duration)
```

### oj agent

Manage agent sessions.

```bash
oj agent logs <pipeline-id>          # View agent logs
oj agent logs <pipeline-id> -s plan  # Filter by step name
oj agent logs <pipeline-id> --follow # Stream logs (alias: -f)
oj agent logs <pipeline-id> -n 100   # Limit lines (default: 50)
oj agent wait <agent-id>             # Wait for agent to idle or exit
oj agent wait <agent-id> --timeout 5m  # With timeout (human-readable duration)
oj agent hook stop <agent-id>        # Claude Code stop hook integration
```

### oj session

Manage execution sessions.

```bash
oj session list
oj session send <id> <input>
oj session peek <id>
oj session attach <id>
```

### oj workspace

Manage isolated work contexts.

```bash
oj workspace list
oj workspace list -n 50             # Limit results (default: 20)
oj workspace list --no-limit        # Show all results
oj workspace show <id>
oj workspace drop [id]              # Delete specific workspace
oj workspace drop --failed          # Delete failed workspaces
oj workspace drop --all             # Delete all workspaces
oj workspace prune                  # Prune merged worktree branches
oj workspace prune --all            # Prune all worktree branches
oj workspace prune --dry-run        # Preview without deleting
```

### oj queue

Manage queues defined in runbooks.

```bash
oj queue list                        # List all known queues
oj queue list -o json                # JSON output
oj queue items <queue>               # Show items in a queue
oj queue items <queue> -o json       # JSON output
oj queue push <queue> '<json>'       # Push item to persisted queue
oj queue drop <queue> <item-id>      # Remove item from queue
oj queue retry <queue> <item-id>     # Retry a dead or failed item
```

Push validates the JSON data against the queue's `vars` and applies `defaults` before writing to the WAL. Pushing to a persisted queue automatically wakes any attached workers.

`oj queue retry` resets a dead or failed item back to pending status, clearing its failure count. The item ID can be a prefix match. The `--project` flag overrides namespace resolution.

### oj worker

Manage workers defined in runbooks.

```bash
oj worker start <name>               # Start a worker (idempotent; wakes if already running)
oj worker list                       # List all workers
oj worker list -o json               # JSON output
```

Workers poll their source queue and dispatch items to their handler pipeline. `oj worker start` is idempotent — it loads the runbook, validates definitions, and begins the poll-dispatch loop. If the worker is already running, it triggers an immediate poll instead.

### oj cron

Manage time-driven daemons defined in runbooks.

```bash
oj cron list                         # List all crons and their status
oj cron list --project <name>        # Filter by project namespace
oj cron start <name>                 # Start a cron (begins interval timer)
oj cron stop <name>                  # Stop a cron (cancels interval timer)
oj cron restart <name>               # Stop, reload runbook, and start
oj cron once <name>                  # Run once now (ignores interval)
oj cron logs <name>                  # View cron activity log
oj cron logs <name> --follow         # Stream logs (alias: -f)
oj cron logs <name> -n 100           # Limit lines (default: 50)
oj cron prune                        # Remove stopped crons from daemon state
```

Crons run their associated pipeline on a recurring schedule. `oj cron start` is idempotent — it loads the runbook, validates the cron definition, and begins the interval timer.

### oj decision

Manage human-in-the-loop decisions.

```bash
oj decision list                     # List pending decisions
oj decision list --project <name>    # Filter by project namespace
oj decision show <id>                # Show details of a decision
oj decision resolve <id>             # Resolve interactively
oj decision resolve <id> 1           # Pick option #1
oj decision resolve <id> -m "msg"    # Resolve with freeform message
```

Decisions are created when pipelines escalate (via `on_idle`, `on_dead`, or `on_error` with `escalate` action) and require human input to continue. Each decision has a context message and optional numbered choices.

## Events

### oj emit

Emit events to the daemon.

```bash
# Signal successful completion (advances pipeline to next step)
oj emit agent:signal --agent <id> '{"kind": "complete"}'

# Signal escalation (pauses pipeline, notifies human)
oj emit agent:signal --agent <id> '{"kind": "escalate", "message": "..."}'
```

The `kind` field also accepts the alias `action`. The JSON payload can also be passed via stdin.

## Namespace Isolation

A single daemon serves all projects. Resources (pipelines, workers, queues) are scoped by a project namespace to prevent collisions. The namespace is resolved in priority order:

1. `--project <name>` flag (on commands that support it)
2. `OJ_NAMESPACE` environment variable (set automatically for nested `oj` calls from agents)
3. `.oj/config.toml` `[project].name` field
4. Directory basename of the project root

When multiple namespaces are present, `oj pipeline list` shows a `PROJECT` column. `oj pipeline show` includes a `Project:` line when the namespace is set.

## JSON Output

Most commands support `-o json` / `--output json` for programmatic use:

```bash
oj pipeline list -o json
oj workspace list -o json
```
