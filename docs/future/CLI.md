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

## ~~oj queue (dead letter)~~ (Implemented)

See [CLI — oj queue](../interface/CLI.md#oj-queue). The implemented command is `oj queue retry` (not `requeue`). A `--dead` filter for `oj queue list` is not yet implemented.

## oj session prune

```bash
oj session prune                 # Kill orphan tmux sessions (no active pipeline)
```
