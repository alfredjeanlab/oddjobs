# Odd Jobs (oj)

An automated team for your odd jobs. Orchestrate work from runbooks.

Odd jobs coordinates multiple AI coding agents with runbook-defined workflows, to plan features, decompose work into issues, execute tasks, and merge results. Agents run in coop sidecars with WebSocket-based lifecycle monitoring. Jobs, queues, and workers provide the coordination primitives.

## Architecture

Everything is defined declaratively in runbooks — commands, jobs, agents, queues, workers, crons. The daemon owns the event loop, persists state to a WAL, and recovers across restarts. Agents run as coop processes with WebSocket lifecycle monitoring, giving users real-time observability and the ability to attach, intervene, or debug running sessions.

```
┌─────────────────────────────────────────────────┐
│              Daemon (ojd)                       │
│                                                 │
│  Runbooks ──→ Engine ──→ Event Loop             │
│                 │                               │
│       ┌─────────┼──────────┐                    │
│       │         │          │                    │
│    Agent    Workspace   Notify                  │
│    (coop)    Adapter    Adapter                 │
│       │                                         │
│       └──→ WebSocket lifecycle monitoring       │
│                                                 │
│  WAL + Snapshots ──→ Crash recovery             │
└─────────────────────────────────────────────────┘
```

## Design Principles

1. **High testability** - Target 95%+ coverage through architectural choices
2. **Composability** - Small modules compose into larger behaviors
3. **Offline-first** - Full functionality without network; sync when available
4. **Observability** - Events and metrics at every boundary
5. **Recoverability** - Checkpoint and resume from any failure

### Building

```bash
cargo build
make check   # Run all CI checks (fmt, clippy, test, build, audit, deny)
```

## License

Licensed under the Business Source License 1.1
Copyright (c) Alfred Jean LLC
See LICENSE for details.
