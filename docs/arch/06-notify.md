# Notifications

How desktop notifications are sent from runbook lifecycle events.

## NotifyAdapter

```rust
#[async_trait]
pub trait NotifyAdapter: Clone + Send + Sync + 'static {
    async fn notify(&self, title: &str, message: &str) -> Result<(), NotifyError>;
}
```

The engine emits `Effect::Notify { title, message }` from runbook `notify {}` blocks on lifecycle events (`on_start`, `on_done`, `on_fail`). The executor calls the adapter; failures are logged but never block job progress.

## DesktopNotifyAdapter

Production implementation using `notify-rust` for native OS notifications.

**macOS considerations**: `notify-rust` calls `Notification::show()` synchronously. The adapter uses `spawn_blocking` to avoid blocking the async runtime. On startup, it pre-sets the application bundle identifier via `mac-notification-sys::set_application` to prevent an `NSAppleScript` lookup that blocks forever in daemon processes lacking Automation permissions.

## FakeNotifyAdapter

Test implementation that records all notifications as `NotifyCall { title, message }` for assertions. No side effects.
