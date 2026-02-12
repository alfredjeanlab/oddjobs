// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

use async_trait::async_trait;
use thiserror::Error;

/// Errors from notify operations
#[derive(Debug, Error)]
pub enum NotifyError {
    #[error("send failed: {0}")]
    SendFailed(String),
}

/// Adapter for sending notifications
#[async_trait]
pub trait NotifyAdapter: Clone + Send + Sync + 'static {
    /// Send a notification with a title and message body
    async fn notify(&self, title: &str, message: &str) -> Result<(), NotifyError>;
}

/// Desktop notification adapter using notify-rust.
///
/// On macOS, `notify-rust` uses `mac-notification-sys` (Cocoa bindings) to send
/// notifications via the Notification Center. The first notification triggers
/// `ensure_application_set()` which runs an AppleScript to look up a bundle
/// identifier. In a daemon context without Automation permissions, that
/// AppleScript blocks forever. We pre-set the bundle identifier at construction
/// time to bypass the lookup entirely.
#[derive(Clone, Copy, Debug, Default)]
pub struct DesktopNotifyAdapter;

impl DesktopNotifyAdapter {
    pub fn new() -> Self {
        #[cfg(target_os = "macos")]
        {
            // Pre-set the application bundle identifier so mac-notification-sys
            // skips its NSAppleScript lookup (which blocks forever in daemon
            // processes that lack Automation permissions).
            let _ = mac_notification_sys::set_application("com.apple.Terminal");
        }
        Self
    }
}

#[async_trait]
impl NotifyAdapter for DesktopNotifyAdapter {
    async fn notify(&self, title: &str, message: &str) -> Result<(), NotifyError> {
        let title = title.to_string();
        let message = message.to_string();
        // notify_rust::Notification::show() is synchronous on macOS.
        // Fire-and-forget on tokio's bounded blocking thread pool to avoid
        // blocking the async runtime while capping OS thread count.
        tokio::task::spawn_blocking(move || {
            tracing::info!(%title, %message, "sending desktop notification");
            match notify_rust::Notification::new().summary(&title).body(&message).show() {
                Ok(_) => {
                    tracing::info!(%title, "desktop notification sent");
                }
                Err(e) => {
                    tracing::warn!(%title, error = %e, "desktop notification failed");
                }
            }
        });
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
#[cfg_attr(coverage_nightly, coverage(off))]
mod fake {
    use super::{NotifyAdapter, NotifyError};
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::sync::Arc;

    /// Recorded notification
    #[derive(Debug, Clone)]
    pub struct NotifyCall {
        pub title: String,
        pub message: String,
    }

    struct FakeNotifyState {
        calls: Vec<NotifyCall>,
    }

    /// Fake notification adapter for testing
    #[derive(Clone)]
    pub struct FakeNotifyAdapter {
        inner: Arc<Mutex<FakeNotifyState>>,
    }

    impl Default for FakeNotifyAdapter {
        fn default() -> Self {
            Self { inner: Arc::new(Mutex::new(FakeNotifyState { calls: Vec::new() })) }
        }
    }

    impl FakeNotifyAdapter {
        pub fn new() -> Self {
            Self::default()
        }

        /// Get all recorded notifications
        pub fn calls(&self) -> Vec<NotifyCall> {
            self.inner.lock().calls.clone()
        }
    }

    #[async_trait]
    impl NotifyAdapter for FakeNotifyAdapter {
        async fn notify(&self, title: &str, message: &str) -> Result<(), NotifyError> {
            self.inner
                .lock()
                .calls
                .push(NotifyCall { title: title.to_string(), message: message.to_string() });
            Ok(())
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use fake::{FakeNotifyAdapter, NotifyCall};

#[cfg(test)]
#[path = "notify_tests.rs"]
mod tests;
