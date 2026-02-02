# CLI — Future Additions

Additions to the CLI described in `docs/02-interface/CLI.md`.

## oj cron

Manage time-driven daemons defined in runbooks.

```bash
oj cron list                     # List all crons and their status
oj cron enable <name>            # Enable a cron
oj cron disable <name>           # Disable a cron
oj cron run <name>               # Run once now (ignores interval)
```

## oj worker stop

```bash
oj worker stop <name>            # Stop a running worker
```

## oj queue (dead letter)

```bash
oj queue list <name> --dead      # List dead letter items
oj queue requeue <name> <id>       # Retry a dead letter item
```

## oj session prune

```bash
oj session prune                 # Kill orphan tmux sessions (no active pipeline)
```
