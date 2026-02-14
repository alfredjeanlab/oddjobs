# Agents

How AI agents run within the oj orchestration system.

## AgentAdapter

Manages agent lifecycle. The engine calls this trait; it never interacts with agent processes directly.

```rust
#[async_trait]
pub trait AgentAdapter: Send + Sync + 'static {
    async fn spawn(&self, config: AgentConfig, event_tx: mpsc::Sender<Event>) -> Result<AgentHandle, AgentAdapterError>;
    async fn send(&self, agent_id: &AgentId, input: &str) -> Result<(), AgentAdapterError>;
    async fn respond(&self, agent_id: &AgentId, response: &PromptResponse) -> Result<(), AgentAdapterError>;
    async fn kill(&self, agent_id: &AgentId) -> Result<(), AgentAdapterError>;
    async fn reconnect(&self, config: AgentReconnectConfig, event_tx: mpsc::Sender<Event>) -> Result<AgentHandle, AgentAdapterError>;
    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentAdapterError>;
    async fn last_message(&self, agent_id: &AgentId) -> Option<String>;
    async fn resolve_stop(&self, agent_id: &AgentId);
    async fn is_alive(&self, agent_id: &AgentId) -> bool;
    async fn capture_output(&self, agent_id: &AgentId, lines: u32) -> Result<String, AgentAdapterError>;
    async fn fetch_transcript(&self, agent_id: &AgentId) -> Result<String, AgentAdapterError>;
    async fn fetch_usage(&self, agent_id: &AgentId) -> Option<UsageData>;
    fn get_coop_host(&self, agent_id: &AgentId) -> Option<CoopInfo> { None }
    fn is_remote_only(&self) -> bool { false }
}
```

The engine holds the adapter as `Arc<dyn AgentAdapter>` (dynamic dispatch) rather than a generic type parameter.

**Production**: `RuntimeRouter` — delegates to the appropriate adapter based on
environment and agent config. Uses a `dispatch!` macro to route operations:

| Adapter | When | Transport |
|---------|------|-----------|
| `LocalAdapter` | No container, daemon running locally | Unix socket |
| `DockerAdapter` | `container` field set, daemon running locally | TCP |
| `KubernetesAdapter` | Daemon running in K8s (all agents route here) | TCP |

When the daemon runs inside a Kubernetes cluster, the router auto-detects via
in-cluster config and routes all agents to `KubernetesAdapter`. Otherwise, it
routes based on whether the agent has a `container` field.

The route for each agent is tracked in memory after spawn. `AgentHandle` returns the `AgentRuntime` (Local/Docker/Kubernetes) and optional `auth_token`, which are persisted in `AgentRecord` for reconnection after daemon restart.

**Test**: `FakeAgentAdapter` — in-memory state, configurable responses, records all calls. Enables deterministic tests, call verification, error injection (`set_spawn_fails(true)`), and state simulation.

Integration tests use [claudeless](https://github.com/anthropics/claudeless) — a CLI simulator that emulates Claude's interface without API costs. The `LocalAdapter` works identically with both real Claude and claudeless.

Source: `crates/daemon/src/adapters/agent/`

## Coop Architecture

Agents run in **coop processes** — PTY-based sidecars that wrap Claude Code, providing session persistence, state detection, and an HTTP/WebSocket control API. The engine communicates with coop over a per-agent Unix socket (local) or TCP (Docker/K8s).

```diagram
Engine                    Coop Sidecar                Claude Code
───────                   ────────────                ───────────
Effect::SpawnAgent ─────→ coop --agent claude ──────→ claude <prompt>
                          │                           │
HTTP API ←──────────────→ socket / TCP :8080          │ (PTY)
WebSocket ←───────────── state events                 │
                          │                           │
send(input) ────────────→ /api/v1/agent/nudge ──────→ keyboard input
respond(prompt) ────────→ /api/v1/agent/respond ────→ prompt answer
kill() ─────────────────→ /api/v1/shutdown ──────────→ graceful exit
```

The same HTTP/WebSocket API is used regardless of transport. For containerized
agents, coop listens on `--port 8080` (TCP) instead of a Unix socket, and a
per-agent bearer token (`COOP_AUTH_TOKEN`) secures each connection. See
[Containers](07-containers.md) for Docker and Kubernetes specifics.

### Why Coop (Not Print Mode)

Agents are long-lived and interactive by design. The coop architecture enables:
- **Observability**: Users can attach to sessions to monitor work in real-time
- **Intervention**: Users can communicate with running agents when needed
- **Persistence**: Sessions survive daemon restarts; reconciliation reconnects
- **Debugging**: Interactive access to diagnose and fix issues

## Spawn Flow

When the engine processes `Effect::SpawnAgent`:

1. **Create agent directory**: `{state_dir}/agt-{agent_id}/`
2. **Write agent-config.json**: Settings, stop gate config, and prime scripts (see below)
3. **Launch coop**: `coop --agent claude --socket {coop.sock} --agent-config {path} -- {command}`
4. **Poll for readiness**: HTTP GET `/api/v1/health` until responsive (~10s timeout)
5. **Start event bridge**: WebSocket subscription for state change events
6. **Emit `AgentSpawned`**: Signal to engine that monitoring is active (includes `AgentRuntime` and optional `auth_token`)

Coop injects `--session-id` and `--agent-config` into the wrapped command automatically based on the agent config.

## Agent Config

The engine writes `agent-config.json` before spawn. Coop reads it to configure the agent:

```json
{
  "settings": { /* merged from workspace .claude/settings.json */ },
  "stop": {
    "mode": "allow"
  },
  "start": {
    "shell": ["set -euo pipefail\n<prime_script>"]
  }
}
```

| Field | Purpose |
|-------|---------|
| **settings** | Project settings merged into Claude's config |
| **stop** | Stop mode — derived from `on_idle` action (see [Stop Hook Flow](#stop-hook-flow)) |
| **start** | SessionStart hook shell commands for prime context injection |

The `start` field can also use per-source primes for different session lifecycle events (startup, resume, clear, compact).

## State Detection

Agent state is detected via coop's WebSocket event bridge. The adapter subscribes to `/ws?subscribe=state,messages` and translates events:

| Coop Event | Engine Event | Description |
|------------|-------------|-------------|
| `transition: working` | `AgentWorking` | Agent processing (tool use, thinking) |
| `transition: idle` | `AgentIdle` | Agent waiting for input |
| `transition: prompt` | `AgentPrompt` | Permission, plan, or question prompt |
| `transition: error` | `AgentFailed` | API error (unauthorized, quota, network) |
| `exit` / WS close | `AgentGone` | Agent process exited |
| `stop:outcome` (blocked) | `AgentStopBlocked` | Stop gate blocked exit |
| `stop:outcome` (allowed) | `AgentStopAllowed` | Stop gate allowed exit |

Coop monitors the agent process directly and reports exit events with the exit code.

## HTTP Control API

The adapter communicates with coop via HTTP over the per-agent Unix socket:

| Operation | Endpoint | Purpose |
|-----------|----------|---------|
| Nudge | POST `/api/v1/agent/nudge` | Send follow-up message (keyboard emulation) |
| Respond | POST `/api/v1/agent/respond` | Answer prompt (permission, plan, question) |
| Kill | POST `/api/v1/shutdown` + `/api/v1/signal` | Graceful then force kill |
| Resolve stop | POST `/api/v1/stop/resolve` | Allow blocked stop to proceed |
| State | GET `/api/v1/agent` | Current state, last message, prompt details |
| Output | GET `/api/v1/screen/text` | Terminal screen capture |
| Transcript | GET `/api/v1/transcripts/catchup` | Full JSONL session transcript |
| Usage | GET `/api/v1/session/usage` | Token counts and cost |
| Health | GET `/api/v1/health` | Liveness check |

## Stop Hook Flow

When an agent finishes a turn, coop's stop hook fires. The behavior depends on the coop stop mode, which is derived from the agent's `on_idle` action:

| `on_idle` | Coop Mode | Behavior |
|-----------|-----------|----------|
| `done` / `fail` | `allow` | Turn ends naturally; engine receives `AgentStopAllowed` and dispatches the on_idle action |
| `nudge` / `gate` / `resume` | `gate` | Coop blocks the exit; engine receives `AgentStopBlocked`, resolves stop, then dispatches on_idle |
| `escalate` | `gate` | Same as above, plus the gate prompt includes "Use the AskUserQuestion tool before proceeding." |
| `auto` | `auto` | Coop handles self-determination; engine does not intervene |

Default on_idle: `done` for job agents, `escalate` for standalone/crew agents.

## Reconnection

On daemon restart, the engine reconciles with surviving agent processes:

1. Attempt `reconnect()` through the `RuntimeRouter` — tries available adapters
   (K8s pod lookup, Docker container inspection, local socket probe)
2. If alive: reconnect starts WebSocket bridge without spawning a new process
3. If dead: emit `AgentGone` to trigger `on_dead` action

This is why daemon shutdown preserves agent processes by default — the restart+reconcile flow picks up exactly where the daemon left off. For containerized agents, pods continue running independently and are rediscovered via the K8s API or Docker inspect.
