# Cloud Execution

Run the daemon in the cloud so jobs survive laptop sleep. Agents run in
containers alongside the daemon in a Kubernetes cluster.

## Use Case

Close your laptop, jobs keep running. Open it in the morning, review results.

```
Today:
  Laptop: CLI → Unix socket → Daemon → tmux (agents)
  Close laptop → everything dies

Cloud:
  Laptop: CLI → TCP → Daemon (cloud) → pods (agents)
  Close laptop → everything keeps running
```

## Deployment

The daemon is lightweight — event loop, WAL, state. A single pod in a
Kubernetes cluster or a $5/month VM is more than enough. The expensive compute
(Claude API calls) happens in agent pods, not the daemon.

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
        env:
        - name: ANTHROPIC_API_KEY
          valueFrom: { secretKeyRef: { name: oj-secrets, key: api-key } }
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

Today: `tmux attach -t <session>` locally.

Over a network, this needs bidirectional terminal streaming — a different
category from request-response.

**MVP: bypass the daemon.** If the user has kubeconfig (they set up K8s), the
CLI runs `kubectl exec -it <pod> -- tmux attach -t main` directly. The daemon
just provides the pod name. No new protocol needed.

```bash
oj attach worker
# → daemon returns pod name via normal request-response
# → CLI execs: kubectl exec -it oj-fix-worker-a1b2 -- tmux attach -t main
```

**Future: WebSocket proxy.** The daemon upgrades a TCP connection to WebSocket
and proxies terminal I/O to the tmux session. Standard approach, but more
infrastructure.

### `oj peek`

Already works — `PeekSession` is a one-shot request-response that returns the
tmux pane capture. No change needed for network.

## Auth

### Unix Socket

No change. Filesystem permissions protect access (existing behavior).

### TCP

Shared secret in the `Hello` handshake:

```json
{"type": "Hello", "version": "0.5.0", "token": "secret-token-here"}
```

The daemon validates the token before processing further requests. Reject with
`{"type": "Error", "message": "unauthorized"}` and close the connection.

Token source: `OJ_AUTH_TOKEN` env var on both CLI and daemon.

Simple, sufficient for single-user deployments. mTLS or OAuth can come later
for multi-user scenarios.

## In-Cluster Networking

When the daemon runs in the cluster, agent-to-daemon IPC becomes trivial.
Agent pods connect to the daemon's Kubernetes Service on the cluster network.

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

Inside agent pods: `OJ_DAEMON_URL=tcp://ojd:7777`. The `oj` CLI in the
container connects over the cluster network. No tunneling, no NAT, no relay.

## What Doesn't Change

- Core engine, state machines, events, effects
- WAL/snapshot persistence (just reads/writes from PVC instead of local disk)
- Runbook parsing
- Job/step/agent lifecycle
- All ~60 IPC request types
- JSON wire format
- Agent adapters (tmux sessions, file watchers — all local to the daemon pod)

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

### Phase 3: Remote Attach

1. `oj attach` queries daemon for pod name
2. CLI runs `kubectl exec -it <pod> -- tmux attach` directly
3. Requires user has kubeconfig (reasonable for K8s users)
