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

## FakeNotifyAdapter

Test implementation that records all notifications as `NotifyCall { title, message }` for assertions. No side effects.
