// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Desktop notification adapter using notify-rust.
//!
//! On macOS, `notify-rust` uses `mac-notification-sys` (Cocoa bindings) to send
//! notifications via the Notification Center. The first notification triggers
//! `ensure_application_set()` which runs an AppleScript to look up a bundle
//! identifier. In a daemon context without Automation permissions, that
//! AppleScript blocks forever. We pre-set the bundle identifier at construction
//! time to bypass the lookup entirely.

use super::{NotifyAdapter, NotifyError};
use async_trait::async_trait;

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
            match notify_rust::Notification::new()
                .summary(&title)
                .body(&message)
                .show()
            {
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
