// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Desktop notification adapter using notify-rust.
//!
//! On macOS, `notify-rust` uses `osascript` (AppleScript) to send notifications
//! via the Notification Center. This requires:
//! - The terminal (or Script Editor) to have notification permissions in
//!   System Settings → Notifications.
//! - The daemon to run in the user's GUI session (not a headless launchd context).

use super::{NotifyAdapter, NotifyError};
use async_trait::async_trait;

#[derive(Clone, Copy, Debug, Default)]
pub struct DesktopNotifyAdapter;

impl DesktopNotifyAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NotifyAdapter for DesktopNotifyAdapter {
    async fn notify(&self, title: &str, message: &str) -> Result<(), NotifyError> {
        let title = title.to_string();
        let message = message.to_string();
        // notify_rust::Notification::show() is synchronous on macOS and may
        // block indefinitely in headless environments. Fire-and-forget in a
        // background thread to avoid blocking the async runtime.
        std::thread::spawn(move || {
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
