# Documentation

```toc
docs/
├── USAGE.md                  # Runbook patterns, best practices, examples
│
├── concepts/                 # What things are
│   ├── RUNBOOKS.md           # Primitives: command, job, agent, cron
│   └── EXECUTION.md          # Workspace and session abstractions
│
├── interface/                # User-facing
│   ├── CLI.md                # Commands and environment variables
│   ├── DECISIONS.md          # Human-in-the-loop decisions
│   ├── EVENTS.md             # Event types and subscriptions
│   └── DESKTOP.md            # Desktop notifications and integration
│
└── arch/                     # Implementation
    ├── 00-overview.md        # Functional core, layers, key decisions
    ├── 01-daemon.md          # Daemon process architecture
    ├── 02-effects.md         # Effect types
    ├── 03-concurrency.md    # Threads, tasks, locks, blocking paths
    ├── 04-storage.md         # WAL persistence
    ├── 05-agents.md          # Agent adapter and coop integration
    └── 06-notify.md          # Desktop notification adapter
```
