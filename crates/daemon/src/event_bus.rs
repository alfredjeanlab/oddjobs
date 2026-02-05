// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Event bus for daemon communication.
//!
//! The EventBus writes events to WAL before notifying the engine,
//! enabling crash recovery via snapshot + replay. Events are buffered in
//! memory and periodically flushed to disk (~10ms durability window).

use oj_core::Event;
use oj_storage::{Wal, WalEntry, WalError};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::mpsc;

/// Event bus backed by WAL.
///
/// Events are written to WAL (buffered) before notifying the engine.
/// This provides durability with group commit for performance.
#[derive(Clone)]
pub(crate) struct EventBus {
    wal: Arc<Mutex<Wal>>,
    wake_tx: mpsc::Sender<()>,
}

/// Reader for the event bus.
///
/// Used by the engine loop to process events from WAL.
pub(crate) struct EventReader {
    wal: Arc<Mutex<Wal>>,
    wake_rx: mpsc::Receiver<()>,
}

impl EventBus {
    /// Create a new event bus backed by the given WAL.
    ///
    /// Returns both the bus (for sending) and reader (for receiving).
    pub fn new(wal: Wal) -> (Self, EventReader) {
        let wal = Arc::new(Mutex::new(wal));
        let (wake_tx, wake_rx) = mpsc::channel(1);

        (
            Self {
                wal: Arc::clone(&wal),
                wake_tx,
            },
            EventReader { wal, wake_rx },
        )
    }

    /// Append event to WAL (buffered, not yet durable).
    ///
    /// Returns the assigned sequence number. Call `flush()` to make durable.
    pub fn send(&self, event: Event) -> Result<u64, WalError> {
        let seq = {
            let mut wal = self.wal.lock();
            wal.append(&event)?
        };
        // Non-blocking wake - if channel is full, engine is already awake
        let _ = self.wake_tx.try_send(());
        Ok(seq)
    }

    /// Flush WAL to disk with single fsync.
    ///
    /// This is the durability point for all buffered events.
    pub fn flush(&self) -> Result<(), WalError> {
        let mut wal = self.wal.lock();
        wal.flush()
    }

    /// Check if WAL needs flushing (interval elapsed or buffer full).
    pub fn needs_flush(&self) -> bool {
        let wal = self.wal.lock();
        wal.needs_flush()
    }

    /// Return the last processed WAL sequence number.
    pub fn processed_seq(&self) -> u64 {
        let wal = self.wal.lock();
        wal.processed_seq()
    }
}

impl EventReader {
    /// Wait for and return next unprocessed event.
    ///
    /// Returns `None` when the bus is closed (all senders dropped).
    pub async fn recv(&mut self) -> Result<Option<WalEntry>, WalError> {
        loop {
            // Check for unprocessed events
            {
                let mut wal = self.wal.lock();
                if let Some(entry) = wal.next_unprocessed()? {
                    return Ok(Some(entry));
                }
            }

            // Wait for wake signal
            if self.wake_rx.recv().await.is_none() {
                // All senders dropped
                return Ok(None);
            }
        }
    }

    /// Mark an entry as processed.
    ///
    /// This updates the in-memory processed_seq. Actual persistence
    /// happens via snapshots.
    pub fn mark_processed(&self, seq: u64) {
        let mut wal = self.wal.lock();
        wal.mark_processed(seq);
    }

    /// Get a clone of the WAL Arc for sharing.
    pub fn wal(&self) -> Arc<Mutex<Wal>> {
        Arc::clone(&self.wal)
    }
}
