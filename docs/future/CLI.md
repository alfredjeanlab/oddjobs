# CLI — Future Additions

Additions to the CLI described in `docs/interface/CLI.md`.

## ~~oj cron~~ (Implemented)

See [CLI — oj cron](../interface/CLI.md#oj-cron). The implemented command also includes `restart`, `logs`, and `prune` subcommands.

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
