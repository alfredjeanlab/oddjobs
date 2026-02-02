# Documentation

```toc
docs/
├── concepts/                 # What things are
│   ├── RUNBOOKS.md           # Primitives: command, pipeline, agent
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
│   ├── 04-storage.md         # WAL persistence
│   ├── 05-adapters.md        # tmux, git, claude adapters
│   └── 06-adapter-claude.md  # How Claude Code runs in sessions
│
└── future/                   # Planned additions (not yet implemented)
    ├── RUNBOOKS.md           # Adds: cron entrypoint, nested pipeline vars
    ├── CLI.md                # Adds: oj cron, worker stop, session prune
    └── runbooks/             # Example HCL runbooks (crons)
```
