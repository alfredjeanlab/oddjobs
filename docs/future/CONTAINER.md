# Containerized Agents

Run agents in Docker containers for local sandboxing. Agents run with
`--dangerously-skip-permissions` and a container limits blast radius without
changing the runbook. Same machine, isolated environment.

## Concepts

**`container`** on an agent or job opts into containerization. No container =
local. Short form takes an image directly; block form allows future extras.

```hcl
container = "coop:claude"

container {
  image = "coop:claude"
  # cpus, memory, etc. later
}
```

**`source`** on a job describes what code the job needs (see SOURCE.md). The
system provisions it — git clone into a Docker volume before the agent starts.

```
No source, no container  →  run in cwd, locally, as the user
source, no container     →  own workspace, locally
no source, container     →  container, no code
source + container       →  container with cloned code
```

## Runbook Surface

```hcl
# Runs locally — crew agent, no isolation needed
agent "mayor" {
  run    = "claude --dangerously-skip-permissions"
  prompt = "Review the backlog"
}

# Runs in a container — sandboxed development agent
agent "worker" {
  container = "coop:claude"
  run       = "claude --dangerously-skip-permissions"
  on_idle   = "done"
}

job "fix" {
  container = "coop:claude"

  source {
    git    = true
    branch = "fix/${source.nonce}"
    ref    = "origin/main"
  }

  step "fix"    { run = { agent = "worker" } }
  step "submit" { run = "git push origin ${source.branch}" }
}
```

## Project Config

```toml
[container]
image = "coop:claude"     # default image
```

Agents/jobs without `container` inherit the project default. If no project
default, only agents with explicit `container` are containerized.

## Architecture

The daemon stays local. It owns the event loop, WAL, state, and IPC. Only
agents and shell steps run in containers. The adapter layer absorbs the
difference — the core engine doesn't know where effects execute.

```
Core (pure)  →  Effect  →  Executor  →  Adapter  →  local / docker
```

The executor is generic over `<A: AgentAdapter, N: NotifyAdapter, ...>`.
A `RuntimeRouter` delegates to the right adapter based on whether the
agent/job has a `container`.

### Why Coop Makes This Simple

Coop already exposes a full HTTP + WebSocket + gRPC API and supports TCP
(`--port`), bearer auth (`--auth-token`), and multi-arch Docker images. The
adapter talks to coop the same way regardless of where the crew — the
only difference is the transport (Unix socket locally, TCP in containers).

```
Local:      LocalAdapter → Unix socket → coop process → claude
Docker:     LocalAdapter → TCP :8080   → coop in container → claude
```

No tmux, no file watchers, no docker exec for interaction, no sidecars for
log forwarding. The `AgentAdapter` trait implementation doesn't change — only
the spawn path and endpoint resolution differ.

## How It Works

Coop runs inside the container. This preserves the entire existing interaction
model — `oj attach`, output capture, input injection, WebSocket state events.
The container image is a superset of the local environment (coop + claude +
git + tools).

```
AgentAdapter method         Docker equivalent
───────────────────────────────────────────────────────────────
spawn(config, tx)           docker run -dit --name <name> ... coop ...
send(id, input)             HTTP POST to coop /api/v1/agent/nudge
kill(id)                    HTTP POST to coop /shutdown + docker rm
is_alive(id)                HTTP GET coop /health
capture_output(id, lines)   HTTP GET coop /api/v1/screen/text
```

### State Detection

Same WebSocket event bridge as local. The adapter subscribes to
`ws://localhost:<port>/ws?subscribe=state,messages` and translates coop events
to engine events. No file watchers, no log tailing, no sidecars.

### Code Provisioning

Code is provisioned via `git clone` into a Docker volume — the same flow that
K8S init containers use (see K8S.md). No bind-mounting local worktrees. This
keeps the Docker and K8S paths consistent: containers are fully isolated from
the host filesystem.

The daemon runs a short-lived "init" container to clone the repo, then starts
the main coop container with the same volume:

```bash
# 1. Create a workspace volume
docker volume create worker-a1b2-ws

# 2. Clone repo into the volume (mirrors K8S init container)
docker run --rm \
  -v worker-a1b2-ws:/workspace \
  -v ~/.ssh:/root/.ssh:ro \
  coop:claude \
  git clone --branch fix/a1b2 --single-branch --depth 1 \
    git@github.com:org/repo.git /workspace

# 3. Start the agent with the cloned workspace
#    Credential is auto-resolved from host (see Credentials section)
docker run -d --name worker-a1b2 \
  -p 9001:8080 \
  -e COOP_AUTH_TOKEN=<generated> \
  -e CLAUDE_CODE_OAUTH_TOKEN=<resolved-from-host> \
  -v worker-a1b2-ws:/workspace \
  coop:claude \
  --port 8080 --agent claude -- claude --dangerously-skip-permissions

# 4. Cleanup after job completes
docker rm worker-a1b2 && docker volume rm worker-a1b2-ws
```

The daemon resolves provisioning from the job's `source` block:

| Source field | Init container behavior |
|---|---|
| `git = true, branch = "...", ref = "origin/main"` | `git clone --branch <branch> --single-branch --depth 1 <repo-url> /workspace` |
| `ref = "origin/main"` (no branch) | Clone + checkout ref. Branch created at ref for shell steps. |
| `source = "folder"` | No clone. Empty volume only. |
| No source | No volume. Container starts with just the image. |

The repo URL is resolved from the project's git remote at job creation time.
SSH keys are mounted read-only from the host for clone authentication.

### Credentials

The daemon auto-resolves credentials from the host and injects them as
environment variables into the container. The fallback chain (matching
coop's credential resolution logic):

```
Flow A — OAuth token (preferred):
  1. CLAUDE_CODE_OAUTH_TOKEN env var
  2. macOS Keychain ("Claude Code-credentials")
  3. ~/.claude/.credentials.json → claudeAiOauth.accessToken

Flow B — API key:
  4. ANTHROPIC_API_KEY env var
  5. ~/.claude/.claude.json → primaryApiKey
```

The daemon walks this chain once at spawn time, then passes the resolved
credential as `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` into the
container. The container image contains no baked-in secrets — credentials
are yoinked from the host at runtime.

For K8S where the host has no credentials, see K8S.md (credentials are
stored as Kubernetes Secrets and injected into the pod spec).

### Auth

The daemon generates a per-agent bearer token and passes it as
`COOP_AUTH_TOKEN` — the only secret injected via environment. The adapter
includes this token in all HTTP/WebSocket requests to the containerized coop.

### Crew Agents (Mayor, Doctor)

Most containerized agents are workers — they receive a task, do it, and exit.
They never call back to the daemon. But crew agents like `mayor` orchestrate
work by calling `oj run ...`, which requires reaching the daemon.

For crew agents in containers, the daemon injects three additional env vars
and the `oj` binary must be present in the image:

| Env var | Purpose |
|---|---|
| `OJ_DAEMON_URL` | TCP endpoint to reach daemon (`tcp://host.docker.internal:7777`) |
| `OJ_AUTH_TOKEN` | Authenticate with daemon over TCP |
| `OJ_PROJECT` | Scope `oj run` calls to the right project |

Crew agents use a custom image that extends `coop:claude` with `oj`:

```hcl
agent "mayor" {
  container = "coop:claude-oj"    # extends coop:claude with oj CLI
  run       = "claude --dangerously-skip-permissions"
  prompt    = "Review the backlog and dispatch work"
}
```

Note: `OJ_STATE_DIR` and `OJ_DAEMON_BINARY` are not injected. These are
local-only concepts — `OJ_STATE_DIR` is for Unix socket discovery (replaced
by `OJ_DAEMON_URL` over TCP) and `OJ_DAEMON_BINARY` enables auto-start
(containers cannot auto-start a daemon; it must already be running).

### One Container Per Job

All steps in a job exec into the same container. Shell steps:
`docker exec <container> bash -c "<command>"`. The container lifecycle matches
the job lifecycle.

## Container Image

The default image is `coop:claude` — coop's published multi-arch image from
`coop/Dockerfile`. It includes coop + Claude Code + common dev tools (git,
python3, build-essential, openssh-client, jq, ripgrep, curl). No `oj` CLI.

Users extend for their stack:

```dockerfile
FROM coop:claude
RUN apt-get update && apt-get install -y rustup nodejs ...
```

For crew agents that need `oj`:

```dockerfile
FROM coop:claude AS claude-oj
COPY oj /usr/local/bin/oj
```

## What Doesn't Change

Compared to local execution, containerized agents use the exact same:

- **Agent control API**: all coop HTTP/WebSocket/gRPC endpoints
- **Event bridge**: WebSocket subscription for state transitions
- **Adapter trait methods**: `spawn`, `send`, `respond`, `kill`, etc.
- **Agent config**: `agent-config.json` with settings, stop/start hooks
- **Stop gate protocol**: coop stop hooks → engine

Only the spawn path (process vs container) and transport (Unix socket vs TCP)
differ.

## Implementation

1. `container` field in runbook parser
2. `DockerAdapter` — spawns coop in Docker containers with `--port`
3. Port allocation (dynamic or sequential from a base port)
4. Source provisioning: init container `git clone` into Docker volume
5. Credential auto-resolution from host (OAuth token / API key fallback chain)
6. Per-agent auth token generation (`COOP_AUTH_TOKEN`)
7. Health check polling over TCP instead of Unix socket
8. Volume + container cleanup on job completion
9. Crew agent support: `OJ_DAEMON_URL` + `OJ_AUTH_TOKEN` + `OJ_PROJECT`
10. `RuntimeRouter` delegates per-agent based on `container` presence
11. `[container]` section in project config with default image inheritance
