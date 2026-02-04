# Odd Jobs (oj)

An automated team for your odd jobs. Orchestrate work from runbooks.

Odd jobs coordinates multiple AI coding agents with runbook-defined workflows, to plan features, decompose work into issues, execute tasks, and merge results. Agents work concurrently with coordination primitives (locks, semaphores, queues) ensuring safe access to shared resources.

## Architecture

**Functional Core, Imperative Shell** - Pure state machines generate effects; adapters execute them:

```
┌────────────────────────────────────────────────┐
│              Imperative Shell                  │
│  ┌──────────────────────────────────────────┐  │
│  │  Engine: Load state, execute effects,    │  │
│  │          persist new state               │  │
│  └──────────────────────────────────────────┘  │
│  ┌─────────┬─────────┬─────────┬─────────┐     │
│  │  tmux   │   git   │ claude  │   wk    │     │
│  │ Adapter │ Adapter │ Adapter │ Adapter │     │
│  └─────────┴─────────┴─────────┴─────────┘     │
└────────────────────────────────────────────────┘
                       │
┌──────────────────────┼─────────────────────────┐
│                      │   Functional Core       │
│  ┌───────────────────┴────────────────────┐    │
│  │  Pipeline, Queue, Task, Lock,          │    │
│  │  Semaphore, Guard state machines       │    │
│  │                                        │    │
│  │  transition(state, event) →            │    │
│  │      (new_state, effects)              │    │
│  └────────────────────────────────────────┘    │
└────────────────────────────────────────────────┘
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
