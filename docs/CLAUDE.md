# Documentation

```example
docs/
├── GUIDE.md                  # Runbook patterns, best practices, examples (planned)
│
├── concepts/                 # What things are
│   ├── RUNBOOKS.md           # Primitives: command, job, agent, cron
│   └── EXECUTION.md          # Workspace and session abstractions
│
├── interface/                # User-facing
│   ├── CLI.md                # Commands and environment variables
│   ├── EVENTS.md             # Event types and subscriptions
│   └── DESKTOP.md            # Desktop notifications and integration
│
├── arch/                     # Implementation
│   ├── 00-overview.md        # Functional core, layers, key decisions
│   ├── 01-daemon.md          # Daemon process architecture
│   ├── 02-effects.md         # Effect types
│   ├── 03-concurrency.md    # Threads, tasks, locks, blocking paths
│   ├── 04-storage.md         # WAL persistence
│   ├── 05-adapters.md        # tmux, git, claude adapters
│   └── 06-adapter-claude.md  # How Claude Code runs in sessions
│
└── future/                   # Planned additions (not yet implemented)
    ├── RUNBOOKS.md           # Adds: nested job vars
    ├── CLI.md                # Adds: worker stop, session prune
    ├── CANCELLATION.md       # Handler cancellation on client disconnect
    ├── CHECKPOINT.md         # Checkpoint lock contention at scale
    ├── CONCURRENCY.md        # Deferred effects, event-driven sequencing
    └── runbooks/             # Example HCL runbooks
```
