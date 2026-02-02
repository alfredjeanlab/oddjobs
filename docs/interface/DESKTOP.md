# Desktop Integration

Cross-platform desktop notifications keep you informed of pipeline events without watching terminals.

## Notifications

The daemon sends desktop notifications for escalation events. Notifications are fired as `Effect::Notify` effects and executed by the engine's executor using `notify_rust::Notification` in a background thread (fire-and-forget, to avoid blocking the executor on macOS where `show()` is synchronous).

| Event | Title | Message |
|-------|-------|---------|
| Pipeline escalated (`on_idle`/`on_dead` escalate) | "Pipeline needs attention: {name}" | trigger (e.g. "on_idle") |
| Gate failed (gate command exits non-zero) | "Pipeline needs attention: {name}" | "gate_failed" |
| Agent signal escalate | "{pipeline_name}" | Agent's escalation message |
| Agent `on_start` | Agent name | Rendered `on_start` template |
| Agent `on_done` | Agent name | Rendered `on_done` template |
| Agent `on_fail` | Agent name | Rendered `on_fail` template |

Notifications use the [notify-rust](https://github.com/hoodie/notify-rust) crate for cross-platform support:

| Platform | Backend |
|----------|---------|
| Linux/BSD | D-Bus (XDG notification spec) |
| macOS | NSUserNotification / UNUserNotificationCenter |
| Windows | WinRT Toast notifications |

### Agent Notifications

Agents support the same `notify {}` block as pipelines to emit desktop notifications on lifecycle events:

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
| `${var.*}` | Pipeline variables (e.g. `${var.env}`) |
| `${pipeline_id}` | Pipeline ID |
| `${name}` | Pipeline name |
| `${agent}` | Agent name |
| `${step}` | Current step name |
| `${error}` | Error message (available in `on_fail`) |

### Notification Settings

On macOS, notifications appear from the `ojd` daemon process. You may need to:
1. Allow notifications from `ojd` in System Settings > Notifications
2. Ensure "Do Not Disturb" is off for notifications to appear

On Linux, ensure a notification daemon is running (most desktop environments include one).

## tmux Integration

Agents run in tmux sessions for persistence and observability. Session names follow the format `oj-{pipeline}-{step}-{random}`, where the `oj-` prefix is added by `TmuxAdapter`, the pipeline and step names are sanitized (invalid characters replaced with hyphens, truncated to 20 and 15 characters respectively), and a 4-character random suffix ensures uniqueness.

```bash
# List all oj sessions
tmux list-sessions | grep '^oj-'

# Attach to a pipeline's active agent session via CLI
oj pipeline attach <pipeline-id>

# Attach to a specific session by ID
oj session attach <session-id>

# Or directly via tmux (session IDs visible in `oj session list`)
tmux attach -t <session-id>
```

The `oj pipeline attach` command looks up the pipeline's current `session_id` and attaches to that tmux session. The `oj pipeline peek` command captures the terminal contents without attaching.
