# Kubernetes Deployment Roadmap

Deficiencies to address before the daemon can reliably run as a pod.

Status: The K8s adapter can create agent pods and communicate with them over TCP.
But the daemon itself was designed as a local user-level process — many
assumptions break when the daemon runs in a pod.

## Critical

### 1. Wire KubernetesAdapter into RuntimeRouter

File: `crates/adapters/src/agent/router.rs`

- Add `Kubernetes` variant to `Route` enum
- Add `k8s: Option<KubernetesAdapter>` field to `RuntimeRouter`
- Add `pub async fn with_k8s(mut self) -> Self` builder that attempts
  `KubernetesAdapter::new().await` (silently falls back if not in a cluster)
- Update `with_log_entry_tx` to forward to K8s adapter if present
- Update routing: when `self.k8s.is_some()`, all agents route to K8s (not just
  containerized ones — there is no "local" inside a K8s pod)
- When `self.k8s.is_none()`, preserve existing Local/Docker routing
- Add `Some(Route::Kubernetes)` arm to all 12 match blocks (send, respond, kill,
  get_state, last_message, resolve_stop, is_alive, capture_output,
  fetch_transcript, fetch_usage, get_coop_info, reconnect)

### 2. Update startup to initialize K8s adapter

File: `crates/daemon/src/lifecycle/startup.rs`

After creating the `RuntimeRouter`, call `.with_k8s().await` before
`.with_log_entry_tx(...)`. This auto-detects whether the daemon is running
inside a cluster (via in-cluster config) and enables K8s routing if so.

### 3. Persist agent route type in WAL

The router's in-memory `Route` map is lost on daemon restart. Persist the route
type (`local`, `docker`, `k8s`) so reconciliation knows which adapter to try
first without probing all of them.

Options:
- Add a `runtime` field to `AgentSpawned` events, materialized into job/crew
  state
- Or store a separate `agent_routes` map in `MaterializedState`

### 4. Fix reconciliation for non-local agents

File: `crates/daemon/src/lifecycle/reconcile.rs`

Three calls to `LocalAdapter::check_alive()` only check local Unix sockets —
won't find K8s agents. Two changes:

- Use the persisted route type (item 3) to call the correct adapter's reconnect
  as the fast path
- Fall back to attempt-based recovery (try all adapters) if the persisted route
  fails — handles edge cases like adapter mismatch after migration
- On success: agent was alive and reconnected
- On error: agent is gone, emit `AgentGone` event
- Remove `use oj_adapters::LocalAdapter` import

## High

### 5. Build script git hash in Docker context

File: `crates/daemon/build.rs`

The daemon embeds `BUILD_GIT_HASH` at compile time. Verify this works in Docker
build context (the `.git` dir needs to be available or the build script needs a
fallback).

### 6. Dockerfile for ojd

New file: `deploy/Dockerfile.ojd`

Multi-stage build following coop's pattern:
- Builder: `rust:1.92-bookworm`, musl cross-compile for static binary
- Runtime: `debian:bookworm-slim` with git, bash, openssh-client, ca-certificates
- `ENV OJ_STATE_DIR=/var/lib/oj`
- `ENTRYPOINT ["ojd"]`

No coop or claude binary needed in the daemon image — agents run in separate
pods using the `coop:claude` image.

### 7. Git remote detection requires a local repo

File: `crates/adapters/src/agent/k8s/mod.rs`

`k8s_spawn` calls `detect_git_remote(&config.project_path)`, which runs
`git remote get-url origin` against a local filesystem path. In a daemon pod,
there is no local checkout — only the PVC with WAL/snapshots. The call fails
silently (returns `None`), so agent pods are created with no init container and
no code.

Similarly, `detect_git_branch_blocking(&config.workspace_path)` checks a local
workspace path that doesn't exist in the daemon pod.

**Fix:** The repo URL and branch must be resolved at job creation time (on the
CLI side or in the runbook) and passed through the event/effect chain — add
`repo_url` and `branch` fields to `AgentConfig`, populated from the `source {}`
block. The daemon pod should never need a local git repo.

### 8. Workspace creation runs on the daemon pod — wasted I/O

File: `crates/engine/src/workspace.rs`

The engine's `Effect::CreateWorkspace` handler creates git worktrees or
directories under `state_dir/workspaces/`. For containerized agents this is
useless — agent pods provision code via init containers. But the daemon still
runs `git worktree add` (fails — no local repo in the pod), creates directories
on the PVC that are never used, and persists `WorkspaceCreated` events with
paths local to the daemon pod.

**Fix:** Skip local workspace creation when the agent will run in a container.
Emit the event (for state tracking) but skip the filesystem operations — the
init container handles provisioning.

### 9. Pod IP addressing is ephemeral — stale after rescheduling

File: `crates/adapters/src/agent/remote.rs`

`RemoteCoopClient` stores agent addresses as raw IP:port strings. Pod IPs are
ephemeral — if an agent pod is evicted and rescheduled, its IP changes. The
daemon has no mechanism to detect or refresh stale addresses. The pod naming
convention (`oj-<agent-id>`) allows re-lookup via the K8s API, but no such
refresh path exists.

**Fix:** On connection failure, attempt a K8s API lookup by pod name to get the
current IP before reporting the agent as dead.

## Medium

### 10. `read_pod_env` shells out to `kubectl`

File: `crates/adapters/src/agent/k8s/mod.rs`

The K8s adapter's `reconnect` path reads the auth token from a running pod by
shelling out to `kubectl exec`. This requires `kubectl` in the daemon container
image and RBAC `exec` permissions (broader than `create`/`delete` pods).

**Fix:** Store the auth token in the daemon's in-memory state (it's already
generated at spawn time) and persist it through the WAL so reconnect doesn't
need to read it from the pod.

### 11. Desktop notifications are macOS-specific

File: `crates/adapters/src/notify.rs`

`DesktopNotifyAdapter` uses `notify-rust` with macOS-specific setup. In a Linux
container with no D-Bus session, all job lifecycle notifications (start, done,
fail) silently fail.

**Fix:** Add a webhook-based notification adapter (HTTP POST to a configurable
URL) for headless environments. Select via environment variable
(`OJ_NOTIFY_WEBHOOK_URL`).

### 12. Logging goes to files, not stdout

File: `crates/daemon/src/main.rs`

The daemon writes logs to `state_dir/daemon.log` with manual rotation. The
tracing subscriber writes exclusively to a file appender. In Kubernetes,
`kubectl logs` shows nothing and log aggregation systems can't collect them.

**Fix:** Add a stdout tracing layer when running in a container. Detect via
environment variable (`OJ_LOG_STDOUT=1`) or the presence of
`KUBERNETES_SERVICE_HOST`. Consider structured JSON output for machine parsing.

### 13. Lock file semantics don't work across nodes

File: `crates/daemon/src/lifecycle/startup.rs`

The daemon uses `fs2` file locking on a PID file stored on the PVC. If the pod
is killed (SIGKILL, OOM, node failure), `shutdown()` never runs, leaving stale
files on the PVC. `flock` semantics vary by PVC backend — NFS supports them, but
EBS and GCE PD do not.

**Fix:** For single-replica deployments, skip file locking when `OJ_TCP_PORT` is
set (K8s ensures only one pod via Deployment `replicas: 1`). For future HA, use
the Kubernetes Lease API for leader election.

### 14. No TLS for TCP transport

File: `crates/daemon/src/main.rs`

The TCP listener and all coop communication use plaintext TCP with a bearer
token in the `Hello` handshake. In a K8s cluster with flat pod networking, any
pod on the network can observe traffic.

**Fix:** Either add optional TLS via `rustls`, or document reliance on a service
mesh (Istio/Linkerd mTLS) for encryption.

### 15. No resource limits on agent pods

File: `crates/adapters/src/agent/k8s/pod.rs`

`build_pod` creates pods with no `resources.requests` or `resources.limits`. In
a shared cluster, agents can starve other workloads and the scheduler can't make
informed placement decisions.

**Fix:** Add configurable resource requests/limits to `PodParams`. Allow override
via runbook `container {}` block or environment variables.

## Low

### 16. No startup probe — aggressive readiness timing

File: `crates/adapters/src/agent/k8s/pod.rs`

The readiness probe starts at 2s with 5s period. Claude Code plus coop startup
can take 15-30s. There's no `startupProbe` to protect the initial boot, so the
kubelet may restart the pod before it's ready. The liveness probe checks for a
Unix socket file that may not exist until coop is fully initialized.

**Fix:** Add a `startupProbe` with generous timing. Change the liveness probe to
HTTP (`/api/v1/health`) instead of socket file existence.

### 17. Graceful shutdown doesn't drain jobs

File: `crates/daemon/src/main.rs`

On SIGTERM, the daemon breaks the event loop immediately. It does not wait for
in-flight effects, signal running agents, or use the K8s
`terminationGracePeriodSeconds` window.

**Fix:** On SIGTERM, enter a drain phase: stop accepting new work, wait for
in-flight effects (with a timeout), then save snapshot and exit. Only important
for shell steps — agent-based jobs survive daemon restarts via reconciliation.

## Implementation Order

1. **K8s adapter wiring + route persistence + reconciliation** (items 1-4) —
   Everything else depends on the daemon knowing how to reach K8s agents after
   restart.
2. **Build script + Dockerfile** (items 5-6) — Produce a runnable daemon image.
3. **Repo URL in event chain** (item 7) — Unblocks K8s agent pods getting code.
4. **Skip local workspace for containers** (item 8) — Cleanup after item 7.
5. **Stdout logging** (item 12) — Quick win for operational visibility.
6. **Webhook notifications** (item 11) — Replaces desktop notifications.
7. **Pod IP refresh** (item 9) — Resilience for long-running agents.
8. **Resource limits** (item 15) — Required before shared-cluster use.
9. **Remove kubectl exec** (item 10) — Auth token stored from spawn, no exec.
10. **Startup probe** (item 16) — Quick pod spec fix.
11. **Lock file** (item 13) — Only matters if lock failures are observed.
12. **TLS** (item 14) — Service mesh can cover this in the interim.
13. **Graceful drain** (item 17) — Nice to have once reconciliation works.
