# Kubernetes Execution

Run the daemon and agents in a Kubernetes cluster. Jobs survive laptop sleep.
Agents run across nodes for fleet parallelism.

## Use Cases

**Persistent execution**: Close your laptop, jobs keep running. Open it in
the morning, review results.

**Fleet parallelism**: Distribute agents across cluster nodes. Remove the
single-machine bottleneck. Ten agents on ten nodes instead of ten agents
fighting for one CPU.

```
Today:
  Laptop: CLI → Unix socket → Daemon → coop (agents)
  Close laptop → everything dies

Kubernetes:
  Laptop: CLI → TCP → Daemon (pod) → coop in pods (agents)
  Close laptop → everything keeps running
```

## Deployment

The daemon is lightweight — event loop, WAL, state. A single pod is more than
enough. The expensive compute (Claude API calls) happens in agent pods, not
the daemon.

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ojd
spec:
  replicas: 1
  template:
    spec:
      containers:
      - name: ojd
        image: ghcr.io/org/ojd:latest
        ports:
        - containerPort: 7777
        volumeMounts:
        - name: state
          mountPath: /var/lib/oj
      volumes:
      - name: state
        persistentVolumeClaim: { claimName: oj-state }
```

From the laptop:

```bash
export OJ_DAEMON_URL=tcp://ojd.mycluster.dev:7777
export OJ_AUTH_TOKEN=secret
oj status
oj run fix "broken tests"
# close laptop, go to sleep
# open laptop next morning
oj status      # job completed overnight
```

## Transport

### Current Protocol

The CLI talks to the daemon over a Unix socket. Wire format:

```
[4-byte BE length][JSON payload]
```

One request-response per connection. ~60 request types, all JSON with a `type`
discriminant. No persistent connections, no streaming. Log following works by
the CLI tailing the log file directly after the daemon returns its path.

### Network Transport

Same wire format over TCP. The daemon listens on both Unix socket (local) and
TCP (remote). The CLI picks the transport based on config:

```
OJ_DAEMON_URL unset         →  Unix socket (existing behavior)
OJ_DAEMON_URL=tcp://host:p  →  TCP connection, same protocol
```

No protocol changes. The ~60 request types, JSON serialization, and
length-prefixed framing all stay identical. The transport layer is the only
difference.

### Log Following

Today the CLI gets a `log_path` from the daemon and tails the file locally.
Over the network, the file isn't accessible.

**Polling with cursor**: The daemon returns content + a byte offset. The CLI
polls with the offset to get new lines. Still request-response, no streaming
needed.

```json
// Request
{"type": "Query", "query": {"type": "GetJobLogs", "id": "abc", "offset": 4096}}

// Response
{"type": "JobLogs", "content": "new lines...", "offset": 5120}
```

The CLI polls at a configurable interval (1-2s). Higher latency than local
file tailing, but functional. Real streaming (WebSocket or long-poll) can come
later if needed.

### `oj attach`

Today: `oj attach` resolves the agent's coop socket and runs
`coop attach <endpoint>` locally.

Over a network, coop processes in pods listen on TCP ports (`--port`)
with full WebSocket support. The daemon proxies the WebSocket connection to
the agent's coop in a standalone thread — the CLI always talks to the daemon,
and the daemon handles connectivity to the agent pod.

```bash
oj attach worker
# → CLI opens WebSocket to daemon
# → daemon spawns proxy thread → coop's /ws?mode=raw in agent pod
# → bidirectional terminal I/O over the proxied connection
```

The proxy runs in a dedicated thread per attachment (not on the event loop).
The daemon knows each agent's coop endpoint (TCP address + auth token), so
no extra configuration is needed.

For advanced setups where the user has direct network access to agent pods
(VPN, ingress, service mesh), the daemon can return the coop endpoint URL
and the CLI runs `coop attach ws://<host>:<port>` directly. This is opt-in
via a configurable proxy strategy.

### `oj peek`

Already works — `PeekSession` calls coop's `GET /api/v1/screen/text` via
the adapter. Over the network, the daemon proxies the same HTTP request.
No change needed.

## Auth

### Unix Socket

No change. Filesystem permissions protect access (existing behavior).

### TCP (Daemon)

Shared secret in the `Hello` handshake:

```json
{"type": "Hello", "version": "0.5.0", "token": "secret-token-here"}
```

The daemon validates the token before processing further requests. Reject with
`{"type": "Error", "message": "unauthorized"}` and close the connection.

Token source: `OJ_AUTH_TOKEN` env var on both CLI and daemon.

### TCP (Coop)

Coop already supports bearer token auth (`--auth-token` / `COOP_AUTH_TOKEN`).
When the daemon spawns a coop process in a pod, it generates a per-agent
token and passes it via `COOP_AUTH_TOKEN`. The adapter includes this token in
all HTTP/WebSocket requests to the containerized coop.

Simple, sufficient for single-user deployments. mTLS or OAuth can come later
for multi-user scenarios.

## Agent Pods

Same coop-inside-container approach as Docker (see CONTAINER.md). The daemon
talks to the Kubernetes API to create and manage pods, and connects to each
coop via TCP on the pod's cluster IP.

```
AgentAdapter method         Kubernetes implementation
───────────────────────────────────────────────────────────────
spawn(config)               Create Pod (coop --port 8080 ...)
                            → adapter connects to <pod-ip>:8080
send(id, input)             HTTP POST coop <pod-ip>:8080 /api/v1/agent/nudge
kill(id)                    HTTP POST coop /shutdown → Delete Pod
is_alive(id)                HTTP GET coop <pod-ip>:8080 /api/v1/health
capture_output(id, lines)   HTTP GET coop <pod-ip>:8080 /api/v1/screen/text
```

### State Detection

WebSocket event bridge to `ws://<pod-ip>:8080/ws?subscribe=state,messages`.
Identical to local and Docker. No sidecars, no log forwarders, no periodic
kubectl exec. Coop's WebSocket is the single observation channel.

### Code Provisioning

No volume mounts across machines. An init container clones the repo before
the main coop container starts. The init container uses the same `coop:claude`
image (it already has git, openssh-client, and ca-certificates from coop's
`base` stage) — no extra image to pull.

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: worker-a1b2
spec:
  initContainers:
  - name: clone
    image: coop:claude
    command: ["git", "clone", "--branch", "fix/a1b2", "--single-branch",
              "--depth", "1", "git@github.com:org/repo.git", "/workspace"]
    volumeMounts:
    - name: workspace
      mountPath: /workspace
    - name: ssh-key
      mountPath: /root/.ssh
      readOnly: true
  containers:
  - name: agent
    image: coop:claude
    args: ["--port", "8080", "--agent", "claude", "--",
           "claude", "--dangerously-skip-permissions"]
    workingDir: /workspace
    ports:
    - containerPort: 8080
    volumeMounts:
    - name: workspace
      mountPath: /workspace
    env:
    - name: COOP_AUTH_TOKEN
      value: "<generated>"
    readinessProbe:
      httpGet: { path: /api/v1/health, port: 8080 }
  volumes:
  - name: workspace
    emptyDir: {}
  - name: ssh-key
    secret: { secretName: oj-deploy-key, defaultMode: 0400 }
```

The daemon resolves provisioning from the job's `source` block at creation time:

| Source field | Init container behavior |
|---|---|
| `git = true, branch = "...", ref = "origin/main"` | `git clone --branch <branch> --single-branch --depth 1 <repo-url> /workspace` |
| `ref = "origin/main"` (no branch) | Clone + checkout ref. Branch created at ref for shell steps. |
| `source = "folder"` | No init container. `emptyDir` volume only. |
| No source | No volume, no init container. |

The repo URL is resolved from the project's git remote at job creation time
on the daemon. SSH deploy keys are provisioned as a Kubernetes Secret and
mounted into the init container.

After the clone completes, the main container starts with `/workspace` as its
working directory. Coop launches Claude, which sees the cloned repo. Shell
steps (`git push`) operate on the same shared volume.

### Credentials

Credentials are injected as environment variables from Kubernetes Secrets.
Unlike Docker (where the daemon auto-resolves from the host), K8S stores
credentials in the cluster and injects them into the pod spec:

```yaml
env:
- name: CLAUDE_CODE_OAUTH_TOKEN       # or ANTHROPIC_API_KEY
  valueFrom:
    secretKeyRef: { name: oj-credentials, key: oauth-token }
```

The daemon references the configured Secret name when creating agent pods.
No credentials are baked into images or stored in the daemon's WAL.

### Auth

Per-agent bearer token via `COOP_AUTH_TOKEN` env var in the pod spec — the
only secret in the pod. The daemon stores the token and includes it in all
HTTP/WebSocket requests to the pod's coop.

### Crew Agents (Mayor, Doctor)

Most pods are workers that never call back to the daemon. Crew agents that
orchestrate work via `oj run ...` need three additional env vars and the `oj`
binary in the image (see CONTAINER.md for the `coop:claude-oj` image):

| Env var | Purpose |
|---|---|
| `OJ_DAEMON_URL` | `tcp://ojd:7777` — daemon Service on cluster network |
| `OJ_AUTH_TOKEN` | Authenticate with daemon over TCP |
| `OJ_PROJECT` | Scope `oj run` calls to the right project |

`OJ_STATE_DIR` and `OJ_DAEMON_BINARY` are not injected — these are local-only
concepts that don't apply in containers (see CONTAINER.md).

### Shell Steps

All steps in a job exec into the same pod. Shell steps run via
`kubectl exec`. Pod lifecycle = job lifecycle.

## Container Image

The default image is `coop:claude` — coop's published multi-arch image from
`coop/Dockerfile`. It includes coop + Claude Code + common dev tools (git,
python3, build-essential, openssh-client, jq, ripgrep, curl). Used for both
the main container and init containers — no separate `alpine/git` image needed.

See CONTAINER.md for extending with project toolchains or `oj` for crew agents.

## In-Cluster Networking

The daemon's Kubernetes Service provides a stable endpoint for crew agents:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: ojd
spec:
  selector:
    app: ojd
  ports:
  - port: 7777
```

Crew agent pods: `OJ_DAEMON_URL=tcp://ojd:7777`. The `oj` CLI in the
container connects over the cluster network. No tunneling, no NAT, no relay.

Worker agent pods don't need this — they have no `oj` CLI and never contact
the daemon. The daemon reaches them via their pod IP and coop's TCP port.

## What Doesn't Change

- Core engine, state machines, events, effects
- WAL/snapshot persistence (just reads/writes from PVC instead of local disk)
- Runbook parsing
- Job/step/agent lifecycle
- All ~60 IPC request types
- JSON wire format
- Coop as the agent control plane (same API, TCP instead of Unix socket)
- Credential injection via environment variables

## Implementation

### Phase 1: TCP Listener

1. Add TCP listener alongside Unix socket in daemon
2. `OJ_DAEMON_URL` in CLI to select transport
3. Same framing, same JSON, same request/response types
4. Auth token in `Hello` handshake

### Phase 2: Remote Log Following

1. Add `offset` field to log query requests
2. Daemon reads from offset, returns new content + new offset
3. CLI polls in a loop for `--follow` mode

### Phase 3: Kubernetes Adapter

1. `KubernetesAdapter` via `kube-rs`
2. Init container for git clone (source provisioning)
3. Pod IP discovery for coop TCP connections
4. Credential injection from Kubernetes Secrets
5. Readiness probe on coop's `/api/v1/health` endpoint
6. Crew agent support: `OJ_DAEMON_URL` + `OJ_AUTH_TOKEN` + `OJ_PROJECT`

### Phase 4: Remote Attach

1. Daemon proxies WebSocket connections to coop in standalone threads
2. CLI opens WebSocket to daemon, daemon forwards to agent's coop `/ws?mode=raw`
3. Optional direct-connect mode returns coop URL for `coop attach` locally
