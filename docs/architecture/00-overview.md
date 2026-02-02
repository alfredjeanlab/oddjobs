# Architecture Overview

## Design Goals

1. **High testability** - 90%+ coverage through architectural choices
2. **Composability** - Small, focused modules that compose
3. **Offline-first** - Full functionality without network
4. **Observability** - Tracing at every boundary (effects, adapters) with entry/exit logging, timing metrics, and precondition validation
5. **Recoverability** - Checkpoint and resume from any failure

## Core Pattern: Functional Core, Imperative Shell

```
┌─────────────────────────────────────────────────────────────────┐
│                       Imperative Shell                          │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐            │
│  │  CLI    │  │  Agent  │  │ Session │  │ Notify  │            │
│  │         │  │ Adapter │  │ Adapter │  │ Adapter │            │
│  └────┬────┘  └────┬────┘  └────┬────┘  └────┬────┘            │
│       │            │            │            │                  │
│  ┌────┴────────────┴────────────┴────────────┴──────────────┐  │
│  │                   Effect Execution Layer                  │  │
│  └─────────────────────────┬─────────────────────────────────┘  │
└────────────────────────────┼────────────────────────────────────┘
                         │
┌────────────────────────┼────────────────────────────────┐
│                        │      Functional Core           │
│  ┌─────────────────────┴───────────────────────────┐    │
│  │              State Machine Engine               │    │
│  │   (Pure state transitions, effect generation)   │    │
│  └─────────────────────┬───────────────────────────┘    │
│                        │                                │
│  ┌─────────┬───────────┘                                │
│  │         │                                            │
│  ▼         ▼                                            │
│ Pipeline  Worker                                        │
│ (pure)    (pure)                                        │
│                                                         │
│  Each module: State + Event → (NewState, Effects)       │
└─────────────────────────────────────────────────────────┘
```

## Module Layers

```
                    ┌──────────┐  ┌──────────┐
                    │   cli    │  │  daemon   │  Layer 4: Entry points
                    └─────┬────┘  └────┬──────┘
                          │            │
                    ┌─────┴────────────┴────┐
                    │        engine         │  Layer 3: Orchestration
                    └──────────┬────────────┘
                               │
          ┌────────────────────┼────────────────────┐
          │                    │                    │
┌─────────▼─────────┐ ┌────────▼────────┐ ┌─────────▼───────┐
│     adapters      │ │     storage     │ │     runbook     │  Layer 2
└───────────────────┘ └─────────────────┘ └────────┬────────┘
          │                    │                    │
          └────────────────────┼────────────────────┘
                               │
          ┌────────────────────┼────────────────────┐
          │                                         │
┌─────────▼─────────┐                    ┌──────────▼────────┐
│        core       │                    │       shell       │  Layer 1: Pure logic
└───────────────────┘                    └───────────────────┘
```

**Dependency Rules:**
1. Higher layers may depend on lower layers
2. Same-layer modules may NOT depend on each other (prevents cycles)
3. `core` depends on serialization, error, ID generation, and sync libraries — but has no I/O
4. `adapters` may use external crates (tokio, process, etc.)

| Layer | Crate | Responsibility | I/O |
|-------|-------|---------------|-----|
| **cli** | `oj` | Parse args, format output, IPC to daemon | stdin/stdout, Unix socket |
| **daemon** | `oj-daemon` | Daemon lifecycle, Unix socket listener, event bus | Unix socket, file I/O |
| **engine** | `oj-engine` | Execute effects, schedule work, runtime handlers | Calls adapters |
| **adapters** | `oj-adapters` | Wrap external tools (claude, tmux, notify) | Subprocess I/O |
| **storage** | `oj-storage` | WAL, snapshots, state materialization | File I/O |
| **runbook** | `oj-runbook` | Parse HCL/TOML, validate, load templates | File read |
| **core** | `oj-core` | Pure state machines, effect generation | None |
| **shell** | `oj-shell` | Shell lexer, parser, AST, validation, execution | Subprocess I/O |

## Key Decisions

### 1. Effects as Data

All side effects are data structures, not function calls:

```rust
enum Effect {
    SpawnAgent { agent_id: AgentId, command: String, ... },
    Emit { event: Event },
    CreateWorkspace { workspace_id: WorkspaceId, path: PathBuf, ... },
    SetTimer { id: TimerId, duration: Duration },
    Shell { pipeline_id: PipelineId, command: String, ... },
    Notify { title: String, message: String },
    // ... see 02-effects.md for full list
}
```

This allows testing without I/O, logging before execution, and dry-run mode.

### 2. Trait-Based Adapters

External integrations go through trait abstractions with production and fake implementations:

| Trait | Production | Test |
|-------|-----------|------|
| `SessionAdapter` | `TmuxAdapter` | `FakeSessionAdapter` / `NoOpSessionAdapter` |
| `AgentAdapter` | `ClaudeAgentAdapter` | `FakeAgentAdapter` |

Production adapters are wrapped with `TracedSession`, `TracedAgent` decorators for observability.

### 3. Event-Driven Architecture

Components communicate via events rather than direct calls, enabling loose coupling. Events flow through an `EventBus` (broadcast) in the daemon, are persisted to a WAL, and processed by the engine's `Runtime`.

### 4. Explicit State Machines

Each primitive has a pure transition function: `(state, event) → (new_state, effects)`

Pipeline and Worker are implemented.

### 5. Injectable Dependencies

Even `core` needs time and IDs, but these must be injectable:

```rust
pub trait Clock: Clone + Send + Sync {
    fn now(&self) -> Instant;
    fn epoch_ms(&self) -> u64;
}
```

Build/integration tests use `SystemClock`; unit tests use `FakeClock` for determinism.

The `IdGen` trait (`UuidIdGen` for production, `SequentialIdGen` for tests) provides deterministic ID generation.

## Data Flow

```
CLI ──parse──▶ Request ──IPC──▶ Daemon (Unix socket)
                                    │
                                    ▼
                   Engine ──▶ Runtime.handle(event) ──▶ (NewState, Effects)
                                                              │
                                ┌─────────────────────────────┘
                                ▼
                      for effect in effects:
                          executor.execute(effect)
                          storage.persist(event)
```

## See Also

- [Daemon](01-daemon.md) - Process architecture (oj + ojd)
- [Effects](02-effects.md) - Effect types and execution
- [Storage](04-storage.md) - WAL and state persistence
- [Adapters](05-adapters.md) - Integration adapters
