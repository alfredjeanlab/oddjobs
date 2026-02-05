# Desktop Integration

Cross-platform desktop notifications keep you informed of job events without watching terminals.

## Notifications

The daemon sends desktop notifications for escalation events. Notifications are fired as `Effect::Notify` effects and executed by the engine's executor using `notify_rust::Notification` in a background thread (fire-and-forget, to avoid blocking the executor on macOS where `show()` is synchronous).

| Event | Title | Message |
|-------|-------|---------|
| Job escalated (`on_idle`/`on_dead` escalate) | "Job needs attention: {name}" | trigger (e.g. "on_idle") |
| Gate failed (gate command exits non-zero) | "Job needs attention: {name}" | "gate_failed" |
| Agent signal escalate | "{job_name}" | Agent's escalation message |
| Job `on_start` | Job name | Rendered `on_start` template |
| Job `on_done` | Job name | Rendered `on_done` template |
| Job `on_fail` | Job name | Rendered `on_fail` template |
| Agent `on_start` | Agent name | Rendered `on_start` template |
| Agent `on_done` | Agent name | Rendered `on_done` template |
| Agent `on_fail` | Agent name | Rendered `on_fail` template |

Notifications use the [notify-rust](https://github.com/hoodie/notify-rust) crate for cross-platform support:

| Platform | Backend |
|----------|---------|
| Linux/BSD | D-Bus (XDG notification spec) |
| macOS | NSUserNotification / UNUserNotificationCenter |
| Windows | WinRT Toast notifications |

### Job Notifications

Jobs support `notify {}` blocks to emit desktop notifications on lifecycle events:

    job "build" {
      name = "${var.name}"
      vars = ["name", "instructions"]

      notify {
        on_start = "Building: ${var.name}"
        on_done  = "Build landed: ${var.name}"
        on_fail  = "Build failed: ${var.name}"
      }
    }

### Agent Notifications

Agents support the same `notify {}` block as jobs to emit desktop notifications on lifecycle events:

    agent "worker" {
      run    = "claude"
      prompt = "Implement the feature."

      notify {
        on_start = "Agent ${agent} started on ${name}"
        on_done  = "Agent ${agent} completed"
        on_fail  = "Agent ${agent} failed: ${error}"
      }
    }

Available template variables:

| Variable | Description |
|----------|-------------|
| `${var.*}` | Job variables (e.g. `${var.env}`) |
| `${job_id}` | Job ID |
| `${name}` | Job name |
| `${agent}` | Agent name |
| `${step}` | Current step name |
| `${error}` | Error message (available in `on_fail`) |

### Notification Settings

On macOS, notifications appear from the `ojd` daemon process. You may need to:
1. Allow notifications from `ojd` in System Settings > Notifications
2. Ensure "Do Not Disturb" is off for notifications to appear

On Linux, ensure a notification daemon is running (most desktop environments include one).

## tmux Integration

Agents run in tmux sessions for persistence and observability. Session names follow the format `oj-{job}-{agent_name}-{random}`, where the `oj-` prefix is added by `TmuxAdapter`, the job and agent names are sanitized (invalid characters replaced with hyphens, truncated to 20 and 15 characters respectively), and a 4-character random suffix ensures uniqueness.

```bash
# List all oj sessions
tmux list-sessions | grep '^oj-'

# Attach to a job's active agent session via CLI
oj job attach <job-id>

# Attach to a specific session by ID
oj session attach <session-id>

# Or directly via tmux (session IDs visible in `oj session list`)
tmux attach -t <session-id>
```

The `oj job attach` command looks up the job's current `session_id` and attaches to that tmux session. The `oj job peek` command captures the terminal contents without attaching.
