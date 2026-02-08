# Cloud Execution (V2)

Run agents and jobs in containers for sandboxing or across a fleet for
parallelism.

## Use Cases

**Local sandboxing**: Agents run with `--dangerously-skip-permissions`. A
container limits blast radius without changing the runbook. Same machine,
isolated environment.

**Fleet parallelism**: Distribute agents across a Kubernetes cluster. Remove
the single-machine bottleneck. Ten agents on ten nodes instead of ten agents
fighting for one CPU.

## Concepts

**`container`** on an agent or job opts into containerization. No container =
local. Short form takes an image directly; block form allows future extras.

```hcl
container = "ghcr.io/org/oj-rust:latest"

container {
  image = "ghcr.io/org/oj-rust:latest"
  # cpus, memory, etc. later
}
```

**`source`** on a job describes what code the job needs (see SOURCE.md). The
system provisions it — worktree locally, clone in a container.

**Backend** in project config decides how containers are run (Docker or
Kubernetes). Runbooks don't know or care.

```
No source, no container  →  run in cwd, locally, as the user
source, no container     →  own workspace, locally
no source, container     →  container, no code
source + container       →  container with code
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
  container = "ghcr.io/org/oj-rust:latest"
  run       = "claude --dangerously-skip-permissions"
  on_idle   = "done"
}

job "fix" {
  container = "ghcr.io/org/oj-rust:latest"

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
backend = "docker"                          # or "kubernetes"
image   = "ghcr.io/org/oj-base:latest"     # default image
```

Agents/jobs without `container` inherit the project default. If no project
default, only agents with explicit `container` are containerized.

## Architecture

The daemon stays local. It owns the event loop, WAL, state, and IPC. Only
agents and shell steps run in containers. The adapter layer absorbs the
difference — the core engine doesn't know where effects execute.

```
Core (pure)  →  Effect  →  Executor  →  Adapter  →  local / docker / k8s
```

The executor is generic over `<S: SessionAdapter, A: AgentAdapter, ...>`.
A `RuntimeRouter` delegates to the right adapter based on whether the
agent/job has a `container` and what backend the project uses.

## Docker Backend

For local sandboxing. Containers run on the host machine.

### How It Works

tmux runs inside the container. This preserves the entire existing interaction
model — `oj attach`, output capture, input injection, session log watching.
The container image is a superset of the local environment (tmux + claude +
git + tools).

```
SessionAdapter method       Docker equivalent
───────────────────────────────────────────────────────────────
spawn(name, cwd, cmd, env)  docker run -dit --name <name> ...
send(id, input)             docker exec <id> tmux send-keys ...
kill(id)                    docker stop <id> && docker rm <id>
is_alive(id)                docker inspect '{{.State.Running}}'
capture_output(id, lines)   docker exec <id> tmux capture-pane ...
```

### Code Provisioning

The daemon creates the worktree locally (existing code), then volume-mounts
it into the container. The file watcher on the host sees Claude's session log
through the bind mount — zero changes to observation code.

### Agent-to-Daemon IPC

Agents call `oj emit agent:signal ...` to signal the daemon. The daemon's Unix
socket is volume-mounted into the container.

### One Container Per Job

All steps in a job exec into the same container. Shell steps:
`docker exec <container> bash -c "<command>"`. The container lifecycle matches
the job lifecycle.

## Kubernetes Backend

For fleet parallelism. Pods run across cluster nodes.

### How It Works

Same tmux-inside-container approach. `kubectl exec` replaces `docker exec`.
The daemon talks to the Kubernetes API to create and manage pods.

```
spawn()           Create Pod
send()            kubectl exec <pod> -- tmux send-keys ...
kill()            Delete Pod
is_alive()        Pod phase == Running
capture_output()  kubectl exec <pod> -- tmux capture-pane ...
```

### Code Provisioning

No volume mounts across machines. The pod clones the repo:

```yaml
initContainers:
- name: clone
  image: alpine/git
  command: ["git", "clone", "--branch", "<branch>", "<repo-url>", "/workspace"]
```

The `source` block's `branch` and `ref` fields drive the clone. The repo URL
is resolved from the project's git remote at job creation time on the daemon.

For `source = "folder"`: empty dir via `emptyDir` volume. No init container.

For no source: no workspace setup. Container starts with just the image.

### Session Log Observation

Local file watchers don't work across machines. Two options:

1. **Log-forwarder sidecar**: Tails Claude's JSONL session log, streams to
   daemon over TCP. Daemon runs a `RemoteWatcher` that consumes the stream.
2. **Periodic fetch**: Daemon periodically `kubectl exec` to read the log.
   Simpler, higher latency.

Sidecar is better for production. Periodic fetch is a viable starting point.

### Agent-to-Daemon IPC

Unix sockets don't cross machines. The daemon exposes a TCP endpoint that
containerized agents connect to. `OJ_DAEMON_URL` environment variable tells
the `oj` CLI inside the container where to connect.

### One Pod Per Job

Same as Docker: all steps in a job exec into the same pod. Shell steps run
via `kubectl exec`. Pod lifecycle = job lifecycle.

For parallel steps (CICD_V1 fan-out): each parallel branch gets its own pod.

## Container Image

Base image with layers. Users extend for their stack:

```dockerfile
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y tmux git curl jq bash
RUN curl -fsSL https://claude.ai/install.sh | bash
COPY oj /usr/local/bin/oj
```

## Implementation Phases

### Phase 1: Docker

1. Container image (tmux + claude + git + oj)
2. `DockerSessionAdapter` (via `bollard` crate or Docker CLI)
3. `container` field in runbook parser
4. Volume-mount workspaces and session logs
5. Volume-mount daemon Unix socket for IPC
6. `oj attach` wraps `docker exec -it`

### Phase 2: Runtime Routing

1. `RuntimeRouter` delegates per-agent based on `container` presence
2. `[container]` section in project config
3. Default image inheritance

### Phase 3: Kubernetes

1. `KubernetesSessionAdapter` via `kube-rs`
2. Init container for git clone
3. Session log observation (sidecar or periodic fetch)
4. TCP endpoint for agent-to-daemon IPC
5. `oj attach` wraps `kubectl exec -it`
