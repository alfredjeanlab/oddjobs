# Kubernetes Deployment Roadmap

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

### 17. Graceful shutdown doesn't drain jobs

File: `crates/daemon/src/main.rs`

On SIGTERM, the daemon breaks the event loop immediately. It does not wait for
in-flight effects, signal running agents, or use the K8s
`terminationGracePeriodSeconds` window.

**Fix:** On SIGTERM, enter a drain phase: stop accepting new work, wait for
in-flight effects (with a timeout), then save snapshot and exit. Only important
for shell steps — agent-based jobs survive daemon restarts via reconciliation.
