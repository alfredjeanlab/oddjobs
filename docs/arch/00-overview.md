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
│  ┌─────────┐  ┌─────────┐  ┌───────────┐  ┌─────────┐         │
│  │  CLI    │  │  Agent  │  │ Workspace │  │ Notify  │         │
│  │         │  │ Adapter │  │  Adapter  │  │ Adapter │         │
│  └────┬────┘  └────┬────┘  └─────┬─────┘  └────┬────┘         │
│       │            │            │            │                 │
│  ┌────┴────────────┴────────────┴────────────┴─────────────┐  │
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
│ Job    Worker   Crew   Cron   Queue   Decision │
│ (pure)                                                  │
│                                                         │
│  Each module: State + Event → (NewState, Effects)       │
└─────────────────────────────────────────────────────────┘
```

## Module Layers

```
                    ┌──────────┐  ┌──────────┐
                    │   cli    │  │  daemon   │  Layer 3: Entry points
                    └─────┬────┘  └────┬──────┘
                          │            │
                    ┌─────┴────────────┴────┐
                    │      daemon internals │  Layer 2: Orchestration + I/O
                    │  engine / adapters /  │
                    │  storage / listener   │
                    └──────────┬────────────┘
                               │
          ┌────────────────────┼────────────────────┐
          │                    │                    │
┌─────────▼─────────┐ ┌────────▼────────┐ ┌─────────▼───────┐
│        core       │ │     runbook     │ │       shell     │  Layer 1: Pure logic
└───────────────────┘ └─────────────────┘ └─────────────────┘
```

**Dependency Rules:**
1. Higher layers may depend on lower layers
2. Same-layer modules may NOT depend on each other (prevents cycles)
3. `core` depends on serialization, error, ID generation, and sync libraries — but has no I/O
4. Daemon internals (adapters, engine, storage) may use external crates (tokio, process, etc.)

| Layer | Crate | Responsibility | I/O |
|-------|-------|---------------|-----|
| **cli** | `oj` | Parse args, format output, IPC to daemon | stdin/stdout, Unix socket |
| **daemon** | `oj-daemon` | Daemon lifecycle, event loop, adapters, engine, storage, listener | Unix socket, TCP, file I/O, subprocess |
| **runbook** | `oj-runbook` | Parse HCL/TOML/JSON, validate, import libraries, load templates | File read |
| **core** | `oj-core` | Pure state machines, effect generation, typed IDs | None |
| **shell** | `oj-shell` | Shell lexer, parser, AST, validation | None |
| **wire** | `oj-wire` | CLI↔daemon IPC protocol types and DTOs | None |

## Key Decisions

### 1. Effects as Data

All side effects are data structures, not function calls:

```rust
enum Effect {
    Emit { event },
    SpawnAgent { agent_id, owner, command, container?, .. },
    SendToAgent { agent_id, input },
    KillAgent { agent_id },
    CreateWorkspace { workspace_id, owner, path, workspace_type?, .. },
    DeleteWorkspace { workspace_id },
    Shell { owner?, command, container?, .. },
    SetTimer { id, duration },
    CancelTimer { id },
    PollQueue { .. },
    TakeQueueItem { .. },
    Notify { title, message },
    ..
}
```

This allows testing without I/O, logging before execution, and dry-run mode. See [Effects](02-effects.md) for the full list.

### 2. Trait-Based Adapters

External integrations go through trait abstractions held as `Arc<dyn Trait>` (dynamic dispatch, not generics). The engine receives all adapters via a `RuntimeDeps` struct:

```rust
pub struct RuntimeDeps {
    pub agents: Arc<dyn AgentAdapter>,
    pub workspace: Arc<dyn WorkspaceAdapter>,
    pub notifier: Arc<dyn NotifyAdapter>,
    pub state: Arc<Mutex<MaterializedState>>,
}
```

| Trait | Production | Test |
|-------|-----------|------|
| `AgentAdapter` | `RuntimeRouter` → Local / Docker / K8s | `FakeAgentAdapter` |
| `WorkspaceAdapter` | `LocalWorkspaceAdapter` | `NoopWorkspaceAdapter` (K8s) |
| `NotifyAdapter` | `DesktopNotifyAdapter` | `FakeNotifyAdapter` |

The `RuntimeRouter` uses a `dispatch!` macro to delegate operations to the correct adapter based on agent config and environment. All adapters live in `crates/daemon/src/adapters/`.

### 3. Event-Driven Architecture

Components communicate via events rather than direct calls, enabling loose coupling. Events flow through an `EventBus` (broadcast) in the daemon, are persisted to a WAL, and processed by the engine's `Runtime`.

### 4. Explicit State Machines

Each primitive has a pure transition function: `(state, event) → (new_state, effects)`

### 5. Injectable Dependencies

Even `core` needs time, but it must be injectable:

```rust
pub trait Clock: Clone + Send + Sync {
    fn now(&self) -> Instant;
    fn epoch_ms(&self) -> u64;
}
```

Build/integration tests use `SystemClock`; unit tests use `FakeClock` for determinism.

### 6. Typed IDs

All entity identifiers use typed wrappers generated by a `define_id!` macro. Each ID is a fixed-size inline buffer (`IdBuf`, 23 bytes: 4-char prefix + 19-char nanoid) that implements `Copy`, avoiding heap allocations:

```rust
define_id! { pub struct JobId("job-"); }
define_id! { pub struct AgentId("agt-"); }
define_id! { pub struct CrewId("crw-"); }
define_id! { pub struct WorkspaceId("wks-"); }
define_id! { pub struct DecisionId("dec-"); }
```

`OwnerId` is a tagged union over `JobId` and `CrewId` — it's also `Copy` and serializes as `"job-XXX"` or `"crw-XXX"`.

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
- [Agents](05-agents.md) - Agent adapter and coop integration
- [Notifications](06-notify.md) - Desktop notification adapter
- [Containers](07-containers.md) - Docker and Kubernetes agent execution
