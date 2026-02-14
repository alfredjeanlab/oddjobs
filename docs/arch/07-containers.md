# Containers

How agents run in Docker containers and Kubernetes pods.

## Overview

The daemon stays local (or runs as a single pod). Only agents and shell steps
run in containers. The adapter layer absorbs the difference — the core engine
doesn't know where effects execute.

```
Core (pure)  →  Effect  →  Executor  →  RuntimeRouter  →  local / docker / k8s
```

## RuntimeRouter

Delegates agent operations to the appropriate adapter based on environment and
agent config.

```rust
enum Route {
    Local,
    Docker,
    Kubernetes,
}
```

**In Kubernetes** (`with_k8s()` succeeds via in-cluster config): all agents
route to `KubernetesAdapter`. There is no "local" inside a pod.

**Locally** (`with_k8s()` falls back silently): agents with a `container` field
route to `DockerAdapter`; agents without route to `LocalAdapter`.

The route for each agent is tracked in memory after spawn and persisted in the
WAL for recovery after daemon restart.

Source: `crates/adapters/src/agent/router.rs`

## Transport

All three adapters use the same coop HTTP/WebSocket API. Only the transport
differs:

| Adapter | Spawn | Transport | Auth |
|---------|-------|-----------|------|
| `LocalAdapter` | `coop --socket {path}` | Unix socket | Filesystem permissions |
| `DockerAdapter` | `docker run ... coop --port 8080` | TCP `localhost:{port}` | Per-agent bearer token |
| `KubernetesAdapter` | Pod via K8s API, `coop --port 8080` | TCP `{pod-ip}:8080` | Per-agent bearer token |

The `RemoteCoopClient` is shared between Docker and Kubernetes adapters — it
implements the 12 `AgentAdapter` trait methods identically once an agent is
registered with an address and token.

Source: `crates/adapters/src/agent/remote.rs`

## Docker Adapter

Runs coop inside Docker containers on the same machine as the daemon.

Source: `crates/adapters/src/agent/docker/`

### Spawn Flow

1. Create a Docker volume for the workspace
2. Run an init container to `git clone` into the volume (if `source { git = true }`)
3. Start the main container with coop listening on `--port 8080`
4. Map a dynamic host port to the container's port 8080
5. Poll `/api/v1/health` until coop is ready
6. Start WebSocket event bridge

### Code Provisioning

Code is provisioned via `git clone` into a Docker volume — the same flow as
Kubernetes init containers. No bind-mounting local worktrees. The daemon resolves
the repo URL from the project's git remote and SSH keys from the host's
`~/.ssh/` directory.

### Credentials

The daemon auto-resolves credentials from the host and injects them as
environment variables. The fallback chain:

```
Flow A — OAuth token (preferred):
  1. CLAUDE_CODE_OAUTH_TOKEN env var
  2. macOS Keychain ("Claude Code-credentials")
  3. ~/.claude/.credentials.json → claudeAiOauth.accessToken

Flow B — API key:
  4. ANTHROPIC_API_KEY env var
  5. ~/.claude/.claude.json → primaryApiKey
```

### Shell Steps

All steps in a job exec into the same container (`docker exec`). Container
lifecycle matches job lifecycle. Cleanup removes both the container and volume.

## Kubernetes Adapter

Runs coop inside Kubernetes pods. The daemon creates pods via the Kubernetes API
(`kube-rs`) and communicates with each coop over TCP on the pod's cluster IP.

Source: `crates/adapters/src/agent/k8s/`

### Spawn Flow

1. Build pod spec with init container, main container, and volumes
2. Create pod via Kubernetes API
3. Wait for pod IP assignment
4. Poll coop `/api/v1/health` until ready
5. Register agent address and start WebSocket event bridge

### Pod Spec

```yaml
spec:
  restartPolicy: Never
  initContainers:
  - name: clone
    image: coop:claude
    command: ["git", "clone", "--branch", "<branch>", "--depth", "1",
              "<repo-url>", "/workspace"]
    volumeMounts:
    - { name: workspace, mountPath: /workspace }
    - { name: ssh-key, mountPath: /root/.ssh, readOnly: true }
  containers:
  - name: agent
    image: coop:claude
    args: ["--port", "8080", "--agent", "claude", "--",
           "claude", "--dangerously-skip-permissions"]
    workingDir: /workspace
    ports: [{ containerPort: 8080 }]
    volumeMounts:
    - { name: workspace, mountPath: /workspace }
    env:
    - { name: COOP_AUTH_TOKEN, value: "<generated>" }
    readinessProbe:
      httpGet: { path: /api/v1/health, port: 8080 }
    livenessProbe:
      exec: { command: ["test", "-S", "/tmp/coop.sock"] }
  volumes:
  - { name: workspace, emptyDir: {} }
  - name: ssh-key
    secret: { secretName: oj-deploy-key, defaultMode: 0400 }
```

Labels: `app=oj-agent`, `oj.dev/agent-id=oj-<agent-id>`.

### Credentials

Injected as environment variables from Kubernetes Secrets (not resolved from
the host):

```yaml
env:
- name: CLAUDE_CODE_OAUTH_TOKEN
  valueFrom:
    secretKeyRef: { name: oj-credentials, key: oauth-token, optional: true }
- name: ANTHROPIC_API_KEY
  valueFrom:
    secretKeyRef: { name: oj-credentials, key: api-key, optional: true }
```

### Crew Agents

Most pods are workers that never call back to the daemon. Crew agents (mayor,
doctor) that orchestrate work via `oj run ...` need three additional env vars
and the `oj` binary in the image:

| Env var | Purpose |
|---------|---------|
| `OJ_DAEMON_URL` | `tcp://ojd:7777` — daemon Service on cluster network |
| `OJ_AUTH_TOKEN` | Authenticate with daemon over TCP |
| `OJ_PROJECT` | Scope `oj run` calls to the right project |

### Kill Flow

1. Send shutdown to coop via HTTP POST `/api/v1/shutdown`
2. Deregister agent from remote client
3. Delete pod via Kubernetes API

## Daemon in Kubernetes

The daemon can run as a pod for persistent execution (jobs survive laptop sleep)
and fleet parallelism (agents distributed across nodes).

### Deployment

Single-replica Deployment with PVC for WAL/snapshot state:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata: { name: ojd }
spec:
  replicas: 1
  template:
    spec:
      containers:
      - name: ojd
        image: ghcr.io/org/ojd:latest
        ports: [{ containerPort: 7777 }]
        volumeMounts:
        - { name: state, mountPath: /var/lib/oj }
      volumes:
      - name: state
        persistentVolumeClaim: { claimName: oj-state }
---
apiVersion: v1
kind: Service
metadata: { name: ojd }
spec:
  selector: { app: ojd }
  ports: [{ port: 7777 }]
```

### Remote Access

The CLI connects over TCP instead of Unix socket:

```bash
export OJ_DAEMON_URL=tcp://ojd.mycluster.dev:7777
export OJ_AUTH_TOKEN=secret
oj status
```

Same wire format (`[4-byte BE length][JSON payload]`), same request types.
Auth token validated in the `Hello` handshake.

### Remote Log Following

Over the network, the CLI can't tail local log files. The daemon returns log
content with a byte offset, and the CLI polls for new content:

```json
{"type": "Query", "query": {"type": "GetJobLogs", "id": "abc", "offset": 4096}}
{"type": "JobLogs", "content": "new lines...", "offset": 5120}
```

### Remote Attach

The daemon proxies WebSocket connections to agent pods in standalone threads.
The CLI always talks to the daemon; the daemon handles connectivity to the
agent's coop endpoint.

## Container Image

The default image is `coop:claude` — coop's multi-arch image with coop + Claude
Code + common dev tools (git, python3, build-essential, openssh-client, jq,
ripgrep, curl). Used for both main and init containers.

Extend for project toolchains:

```dockerfile
FROM coop:claude
RUN apt-get update && apt-get install -y rustup nodejs ...
```

For crew agents that need `oj`:

```dockerfile
FROM coop:claude AS claude-oj
COPY oj /usr/local/bin/oj
```

## Environment Variables

### Daemon (K8s deployment)

| Variable | Purpose | Default |
|----------|---------|---------|
| `OJ_STATE_DIR` | State directory (WAL, snapshots) | `~/.local/state/oj` |
| `OJ_TCP_PORT` | TCP listener port | (disabled) |
| `OJ_AUTH_TOKEN` | Token for TCP auth | (required if TCP) |
| `OJ_K8S_NAMESPACE` | Namespace for agent pods | `default` |
| `OJ_K8S_IMAGE` | Container image for agents | `coop:claude` |
| `OJ_K8S_CREDENTIAL_SECRET` | K8s Secret for API keys | (optional) |
| `OJ_K8S_SSH_SECRET` | K8s Secret for deploy keys | (optional) |
| `OJ_K8S_COOP_PORT` | Coop port in container | `8080` |

### Daemon (Docker deployment)

| Variable | Purpose | Default |
|----------|---------|---------|
| `OJ_DOCKER_IMAGE` | Container image for agents | `coop:claude` |
| `OJ_DOCKER_BASE_PORT` | Starting port for mapping | `9100` |

### CLI (remote daemon)

| Variable | Purpose |
|----------|---------|
| `OJ_DAEMON_URL` | `tcp://host:port` (unset = Unix socket) |
| `OJ_AUTH_TOKEN` | Token for TCP auth |

## What Doesn't Change

Compared to local execution, containerized agents use the exact same:

- Core engine, state machines, events, effects
- Agent control API (all coop HTTP/WebSocket endpoints)
- Event bridge (WebSocket subscription for state transitions)
- Adapter trait methods (spawn, send, respond, kill, etc.)
- Stop gate protocol
- WAL/snapshot persistence
- Runbook parsing
- All ~60 IPC request types and JSON wire format

Only the spawn path (process vs container vs pod), transport (Unix socket vs
TCP), and credential source (host vs Secret) differ.
