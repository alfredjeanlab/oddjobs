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

## Convenience Commands

Top-level shortcuts that auto-detect entity type (job, agent, or queue) by ID prefix:

```bash
oj show <id>                   # Show details of a job, agent, or queue
oj show <id> -v                # Show full variable values (no truncation)
oj peek <id>                   # Peek at the active agent session
oj attach <id>                 # Attach to an agent session
oj logs <id>                   # View logs for a job or agent
oj logs <id> --follow          # Stream logs (alias: -f)
oj logs <id> -n 100            # Limit lines (default: 50)
oj cancel <ids...>             # Cancel one or more running jobs
oj suspend <ids...>            # Suspend one or more running jobs
oj resume [id]                 # Resume an escalated job
oj resume -m "message"         # Resume with nudge/recovery message
oj resume --all                # Resume all resumable jobs
oj status                      # Overview of active work across all projects
```

## Daemon

### oj daemon

Manage the background daemon.

```bash
oj daemon start              # Start daemon (background)
oj daemon start --foreground # Start in foreground (debugging)
oj daemon stop               # Graceful shutdown (sessions preserved)
oj daemon stop --kill        # Stop and terminate all sessions
oj daemon restart             # Stop and restart
oj daemon restart --kill      # Kill sessions then restart
oj daemon status             # Health check
oj daemon logs               # View daemon logs (default: 200 lines)
oj daemon logs --follow      # Stream logs (alias: -f)
oj daemon logs -n 100        # Show last N lines
oj daemon logs --no-limit    # Show all lines
oj daemon orphans            # List orphaned jobs from startup
oj daemon orphans --dismiss <id>  # Dismiss an orphan
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

When listing commands, `oj run` shows warnings for any runbook files that failed to parse, helping diagnose missing commands.

## Resources

### oj job

Manage running jobs.

```bash
oj job list                     # List all jobs
oj job list build               # Filter by name substring
oj job list --status running    # Filter by status
oj job list -n 50               # Limit results (default: 20)
oj job list --no-limit          # Show all results
oj job show <id>                # Shows Project: field when project is set
oj job show <id> -v             # Full variable values (no truncation)
oj job resume [id]              # Resume an escalated job
oj job resume <id> -m "message" --var key=value
oj job resume --all             # Resume all resumable jobs
oj job cancel <ids...>          # Cancel one or more running jobs
oj job suspend <ids...>         # Suspend running jobs (preserves workspace)
oj job attach <id>              # Attach to active agent session
oj job peek <id>                # View agent session output
oj job logs <id>                # View job logs
oj job logs <id> --follow       # Stream logs (alias: -f)
oj job logs <id> -n 100         # Limit lines (default: 50)
oj job prune                    # Remove old terminal jobs
oj job prune --all              # Remove all terminal jobs regardless of age
oj job prune --failed           # Remove all failed jobs
oj job prune --orphans          # Prune orphaned jobs
oj job prune --dry-run          # Preview without deleting
oj job wait <ids...>            # Wait for job completion
oj job wait <id> --timeout 30m  # With timeout (human-readable duration)
oj job wait --all               # Wait for ALL jobs (default: ANY)
```

### oj agent

Manage agent sessions.

```bash
oj agent list                         # List agents across all jobs
oj agent list --job <id>              # Filter by job ID
oj agent list --status running        # Filter by status
oj agent show <id>                    # Show detailed agent info
oj agent send <agent-id> <message>    # Send a message to a running agent
oj agent logs <id>                    # View agent logs
oj agent logs <id> -s plan            # Filter by step name
oj agent logs <id> --follow           # Stream logs (alias: -f)
oj agent logs <id> -n 100             # Limit lines (default: 50)
oj agent peek <id>                    # Peek at agent's session output
oj agent attach <id>                  # Attach to agent's session
oj agent kill <id>                    # Kill agent's session (triggers on_dead)
oj agent suspend <id>                 # Suspend agent's job
oj agent resume [id]                  # Resume a dead agent's session
oj agent resume --all                 # Resume all dead agents
oj agent resume --kill                # Force kill before resuming
oj agent wait <agent-id>              # Wait for agent to idle or exit
oj agent wait <agent-id> --timeout 5m # With timeout (human-readable duration)
oj agent prune                        # Remove agent logs from terminal jobs
oj agent prune --dry-run              # Preview without deleting
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
oj queue show <queue>                # Show items in a queue
oj queue show <queue> -o json        # JSON output
oj queue push <queue> '<json>'       # Push item to persisted queue
oj queue push <queue> --var k=v      # Push item with --var flags
oj queue drop <queue> <item-id>      # Remove item from queue
oj queue retry <queue> [item-ids...] # Retry dead or failed items
oj queue retry <queue> --all-dead    # Retry all dead items
oj queue fail <queue> <item-id>      # Force-fail an active item
oj queue done <queue> <item-id>      # Force-complete an active item
oj queue drain <queue>               # Remove and return all pending items
oj queue logs <queue>                # View queue activity log
oj queue logs <queue> --follow       # Stream logs (alias: -f)
oj queue prune <queue>               # Remove completed/dead items
oj queue prune <queue> --dry-run     # Preview without deleting
```

Push validates the JSON data against the queue's `vars` and applies `defaults` before writing to the WAL. Pushing to a persisted queue automatically wakes any attached workers.

`oj queue retry` resets dead or failed items back to pending status, clearing failure counts. Item IDs support prefix matching.

### oj worker

Manage workers defined in runbooks.

```bash
oj worker start <name>               # Start a worker (idempotent; wakes if already running)
oj worker start --all                # Start all workers defined in runbooks
oj worker stop <name>                # Stop a worker (active jobs continue)
oj worker stop --all                 # Stop all running workers
oj worker restart <name>             # Stop, reload runbook, and start
oj worker resize <name> <n>          # Resize concurrency at runtime
oj worker list                       # List all workers
oj worker list -o json               # JSON output
oj worker logs <name>                # View worker activity log
oj worker logs <name> --follow       # Stream logs (alias: -f)
oj worker prune                      # Remove stopped workers from state
oj worker prune --dry-run            # Preview without deleting
```

Workers poll their source queue and dispatch items to their handler job. `oj worker start` is idempotent — it loads the runbook, validates definitions, and begins the poll-dispatch loop. If the worker is already running, it triggers an immediate poll instead.

### oj cron

Manage time-driven daemons defined in runbooks.

```bash
oj cron list                         # List all crons and their status
oj cron list --project <name>        # Filter by project
oj cron start <name>                 # Start a cron (begins interval timer)
oj cron start --all                  # Start all crons defined in runbooks
oj cron stop <name>                  # Stop a cron (cancels interval timer)
oj cron stop --all                   # Stop all running crons
oj cron restart <name>               # Stop, reload runbook, and start
oj cron once <name>                  # Run once now (ignores interval)
oj cron logs <name>                  # View cron activity log
oj cron logs <name> --follow         # Stream logs (alias: -f)
oj cron logs <name> -n 100           # Limit lines (default: 50)
oj cron prune                        # Remove stopped crons from daemon state
oj cron prune --dry-run              # Preview without deleting
```

Crons run their associated job on a recurring schedule. `oj cron start` is idempotent — it loads the runbook, validates the cron definition, and begins the interval timer.

### oj decision

Manage human-in-the-loop decisions.

```bash
oj decision list                     # List pending decisions
oj decision list --project <name>    # Filter by project namespace
oj decision show <id>                # Show details of a decision
oj decision review                   # Interactively review pending decisions
oj decision resolve <id> 1           # Pick option #1
oj decision resolve <id> -m "msg"    # Resolve with freeform message
```

Decisions are created when jobs escalate and require human input to continue. See [DECISIONS.md](DECISIONS.md) for sources, option mapping, and lifecycle.

### oj project

Manage projects.

```bash
oj project list                      # List projects with active work
```

### oj runbook

Manage runbooks and libraries.

```bash
oj runbook list                      # List runbooks for current project
oj runbook search [query]            # Search available libraries
oj runbook info <path>               # Show library contents and parameters
oj runbook add <path>                # Install an HCL library
oj runbook add <path> --name ns/lib  # Install with explicit name
oj runbook add <path> --project      # Install to project-level (not user-level)
```

See [Runbooks](../concepts/RUNBOOKS.md) for library and import system details.

## Namespace Isolation

A single daemon serves all projects. Resources (jobs, workers, queues) are scoped by a project namespace to prevent collisions. The project is resolved in priority order:

1. `--project <name>` flag (on commands that support it)
2. `OJ_PROJECT` environment variable (set automatically for nested `oj` calls from agents)
3. `.oj/config.toml` `[project].name` field
4. Directory basename of the project root

When multiple namespaces are present, `oj job list` shows a `PROJECT` column. `oj job show` includes a `Project:` line when the project is set.

## Environment Variables

### Daemon Connection

| Variable | Purpose |
|----------|---------|
| `OJ_DAEMON_URL` | Remote daemon endpoint (`tcp://host:port`). Unset = Unix socket. |
| `OJ_AUTH_TOKEN` | Bearer token for TCP auth (required when `OJ_DAEMON_URL` is set) |
| `OJ_PROJECT` | Project scope override (auto-set for nested `oj` calls from agents) |

### Container Configuration

| Variable | Purpose | Default |
|----------|---------|---------|
| `OJ_DOCKER_IMAGE` | Default container image (Docker) | `coop:claude` |
| `OJ_DOCKER_BASE_PORT` | Starting port for container port mapping | `9100` |
| `OJ_K8S_NAMESPACE` | Kubernetes namespace for agent pods | `default` |
| `OJ_K8S_IMAGE` | Container image for K8s agents | `coop:claude` |
| `OJ_K8S_CREDENTIAL_SECRET` | K8s Secret name for API keys | (optional) |
| `OJ_K8S_SSH_SECRET` | K8s Secret name for deploy keys | (optional) |
| `OJ_K8S_COOP_PORT` | Coop port inside agent containers | `8080` |

### Daemon Configuration

| Variable | Purpose | Default |
|----------|---------|---------|
| `OJ_STATE_DIR` | State directory (WAL, snapshots, logs) | `~/.local/state/oj` |
| `OJ_TCP_PORT` | Enable TCP listener on this port | (disabled) |
| `OJ_IPC_TIMEOUT_MS` | IPC timeout in milliseconds | `5000` |
| `OJ_TIMER_CHECK_MS` | Timer resolution in milliseconds | `1000` |

## JSON Output

Most commands support `-o json` / `--output json` for programmatic use:

```bash
oj job list -o json
oj workspace list -o json
```
