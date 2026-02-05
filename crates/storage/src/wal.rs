// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! JSONL event write-ahead log with group commit support.
//!
//! Events are durably stored before processing, enabling crash recovery
//! via snapshot + replay. Group commit batches writes (~10ms) for performance.
//!
//! Each entry is a single line of JSON: `{"seq":N,"event":{...}}\n`

use oj_core::Event;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::warn;

/// Flush interval for group commit (~10ms batches)
const FLUSH_INTERVAL: Duration = Duration::from_millis(10);

/// Maximum entries to buffer before forcing flush
const FLUSH_THRESHOLD: usize = 100;

/// Errors that can occur in Wal operations
#[derive(Debug, Error)]
pub enum WalError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Corrupt entry at offset {offset}: {message}")]
    Corrupt { offset: u64, message: String },
}

/// Serialization helper for writing WAL entries without cloning the event.
#[derive(Serialize)]
struct WalRecordRef<'a> {
    seq: u64,
    event: &'a Event,
}

/// Deserialization helper for reading WAL entries.
#[derive(Deserialize)]
struct WalRecord {
    seq: u64,
    event: Event,
}

/// A single WAL entry with sequence number
#[derive(Debug, Clone)]
pub struct WalEntry {
    pub seq: u64,
    pub event: Event,
}

/// JSONL WAL for durable event storage with group commit.
///
/// Events are buffered in memory and flushed to disk either:
/// - When `needs_flush()` returns true (interval elapsed or buffer full)
/// - Explicitly via `flush()`
///
/// The WAL tracks both the write sequence (highest seq written) and
/// processed sequence (highest seq the engine has processed).
pub struct Wal {
    file: File,
    /// Persistent read handle (cloned once at open) for next_unprocessed
    read_file: File,
    path: PathBuf,
    /// Next sequence number to assign
    write_seq: u64,
    /// Sequence number of last processed entry
    processed_seq: u64,
    /// Buffered JSON lines waiting to be flushed (without trailing newline)
    write_buffer: Vec<Vec<u8>>,
    /// Last flush timestamp for interval checking
    last_flush: Instant,
    /// Current read position for next_unprocessed
    read_offset: u64,
}

impl Wal {
    /// Open or create a WAL at the given path.
    ///
    /// The `processed_seq` should come from the snapshot (or 0 if no snapshot).
    /// The WAL will scan to find the write_seq and set read_offset appropriately.
    pub fn open(path: &Path, processed_seq: u64) -> Result<Self, WalError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)?;

        // Scan to find max sequence and build offset index
        let (mut write_seq, mut read_offset, corrupt) = Self::scan_wal(&file, processed_seq)?;

        if corrupt {
            // Collect valid entries before corruption
            let valid_lines = Self::read_valid_lines(&file)?;

            // Drop file handle before rename
            drop(file);

            // Rotate corrupt WAL to .bak
            let bak_path = crate::snapshot::rotate_bak_path(path);
            warn!(
                path = %path.display(),
                bak = %bak_path.display(),
                valid_entries = valid_lines.len(),
                "Corrupt WAL detected, rotating to .bak and preserving valid entries",
            );
            std::fs::rename(path, &bak_path)?;

            // Create new clean WAL with only valid entries
            {
                let mut new_file = File::create(path)?;
                for line in &valid_lines {
                    new_file.write_all(line.as_bytes())?;
                    new_file.write_all(b"\n")?;
                }
                new_file.sync_all()?;
            }

            // Re-open the clean file
            file = OpenOptions::new()
                .create(true)
                .read(true)
                .append(true)
                .open(path)?;

            // Re-scan the clean file for correct offsets
            let scan = Self::scan_wal(&file, processed_seq)?;
            write_seq = scan.0;
            read_offset = scan.1;
        }

        let read_file = file.try_clone()?;

        Ok(Self {
            file,
            read_file,
            path: path.to_owned(),
            write_seq,
            processed_seq,
            write_buffer: Vec::new(),
            last_flush: Instant::now(),
            read_offset,
        })
    }

    /// Scan the WAL to find the maximum sequence number and offset for processed_seq.
    ///
    /// Returns `(max_seq, read_offset, corrupt)` where `corrupt` is true if
    /// a parse error was encountered (not just EOF).
    fn scan_wal(file: &File, processed_seq: u64) -> Result<(u64, u64, bool), WalError> {
        let mut reader = BufReader::new(file.try_clone()?);
        reader.seek(SeekFrom::Start(0))?;

        let mut max_seq = 0u64;
        let mut read_offset = 0u64;
        let mut current_offset = 0u64;
        let mut corrupt = false;
        let mut line = String::new();

        loop {
            line.clear();
            let bytes_read = match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                    corrupt = true;
                    break;
                }
                Err(e) => return Err(e.into()),
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                current_offset += bytes_read as u64;
                continue;
            }

            // Parse to extract seq; treat parse failure as corruption
            let record: WalRecord = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(_) => {
                    corrupt = true;
                    break;
                }
            };

            max_seq = max_seq.max(record.seq);

            // Track offset of entry after processed_seq (for reading unprocessed)
            if record.seq > processed_seq && read_offset == 0 {
                read_offset = current_offset;
            }

            current_offset += bytes_read as u64;
        }

        // If no unprocessed entries found, read_offset is at end of file
        if read_offset == 0 {
            read_offset = current_offset;
        }

        Ok((max_seq, read_offset, corrupt))
    }

    /// Read all valid (parseable) lines from the WAL, stopping at the first corrupt entry.
    fn read_valid_lines(file: &File) -> Result<Vec<String>, WalError> {
        let mut reader = BufReader::new(file.try_clone()?);
        reader.seek(SeekFrom::Start(0))?;

        let mut valid_lines = Vec::new();
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::InvalidData => break,
                Err(e) => return Err(e.into()),
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Stop at first unparseable entry
            let _: WalRecord = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(_) => break,
            };

            valid_lines.push(trimmed.to_string());
        }

        Ok(valid_lines)
    }

    /// Append an event to the write buffer.
    ///
    /// Returns the assigned sequence number. The event is NOT durable until
    /// `flush()` is called.
    pub fn append(&mut self, event: &Event) -> Result<u64, WalError> {
        self.write_seq += 1;
        let seq = self.write_seq;
        let record = WalRecordRef { seq, event };
        let json_bytes = serde_json::to_vec(&record)?;
        self.write_buffer.push(json_bytes);
        Ok(seq)
    }

    /// Check if flush is needed (interval elapsed or buffer full).
    pub fn needs_flush(&self) -> bool {
        !self.write_buffer.is_empty()
            && (self.last_flush.elapsed() >= FLUSH_INTERVAL
                || self.write_buffer.len() >= FLUSH_THRESHOLD)
    }

    /// Flush all buffered entries to disk with a single fsync.
    ///
    /// This is the durability point - after flush returns successfully,
    /// all buffered events are guaranteed to be on disk.
    pub fn flush(&mut self) -> Result<(), WalError> {
        if self.write_buffer.is_empty() {
            return Ok(());
        }

        for mut json_bytes in self.write_buffer.drain(..) {
            json_bytes.push(b'\n');
            self.file.write_all(&json_bytes)?;
        }

        self.file.sync_all()?;
        self.last_flush = Instant::now();
        Ok(())
    }

    /// Get the next unprocessed entry from the WAL.
    ///
    /// Returns `None` if all entries have been processed or no entries exist.
    pub fn next_unprocessed(&mut self) -> Result<Option<WalEntry>, WalError> {
        // First flush any pending writes so they're readable
        self.flush()?;

        let mut reader = BufReader::new(&self.read_file);
        reader.seek(SeekFrom::Start(self.read_offset))?;

        let mut line = String::new();
        let bytes_read = match reader.read_line(&mut line) {
            Ok(0) => return Ok(None),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::InvalidData => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let record: WalRecord = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    offset = self.read_offset,
                    error = %e,
                    "Corrupt WAL entry, skipping",
                );
                // Advance past the corrupt line to avoid getting stuck
                self.read_offset += bytes_read as u64;
                return Ok(None);
            }
        };

        self.read_offset += bytes_read as u64;

        Ok(Some(WalEntry {
            seq: record.seq,
            event: record.event,
        }))
    }

    /// Mark an entry as processed.
    ///
    /// This updates the in-memory processed_seq. The actual persistence
    /// of this value happens via snapshots.
    pub fn mark_processed(&mut self, seq: u64) {
        self.processed_seq = seq;
    }

    /// Get the current processed sequence number.
    pub fn processed_seq(&self) -> u64 {
        self.processed_seq
    }

    /// Get the current write sequence number.
    pub fn write_seq(&self) -> u64 {
        self.write_seq
    }

    /// Truncate entries before the given sequence number.
    ///
    /// This is called after checkpoint to reclaim disk space.
    /// Creates a new WAL file with only entries >= seq.
    pub fn truncate_before(&mut self, seq: u64) -> Result<(), WalError> {
        // Ensure all writes are flushed first
        self.flush()?;

        let tmp_path = self.path.with_extension("tmp");

        // Read lines from the current file, keeping those with seq >= target
        let mut reader = BufReader::new(self.file.try_clone()?);
        reader.seek(SeekFrom::Start(0))?;

        let mut kept_lines: Vec<(u64, String)> = Vec::new();
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::InvalidData => break,
                Err(e) => return Err(e.into()),
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Parse to check seq
            let record: WalRecord = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(_) => break,
            };

            if record.seq >= seq {
                // Keep this line as raw bytes (no re-serialize)
                kept_lines.push((record.seq, trimmed.to_string()));
            }
        }

        // Write to temp file, computing read_offset during the write pass
        let new_read_offset;
        {
            let mut tmp_file = File::create(&tmp_path)?;
            let mut current_offset = 0u64;
            let mut found_unprocessed = false;
            let mut first_unprocessed_offset = 0u64;

            for (entry_seq, kept_line) in &kept_lines {
                if *entry_seq > self.processed_seq && !found_unprocessed {
                    first_unprocessed_offset = current_offset;
                    found_unprocessed = true;
                }
                tmp_file.write_all(kept_line.as_bytes())?;
                tmp_file.write_all(b"\n")?;
                current_offset += kept_line.len() as u64 + 1;
            }

            // If no unprocessed entries found, read_offset is at end of file
            new_read_offset = if found_unprocessed {
                first_unprocessed_offset
            } else {
                current_offset
            };

            tmp_file.sync_all()?;
        }

        // Atomic rename
        std::fs::rename(&tmp_path, &self.path)?;

        // Reopen file
        self.file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)?;
        self.read_file = self.file.try_clone()?;
        self.read_offset = new_read_offset;

        Ok(())
    }

    /// Iterate over all entries after the given sequence number.
    ///
    /// Used for recovery (replaying from snapshot) and truncation.
    pub fn entries_after(&self, seq: u64) -> Result<Vec<WalEntry>, WalError> {
        let mut reader = BufReader::new(self.file.try_clone()?);
        reader.seek(SeekFrom::Start(0))?;

        let mut entries = Vec::new();
        let mut line = String::new();
        let mut current_offset = 0u64;

        loop {
            line.clear();
            let bytes_read = match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::InvalidData => break,
                Err(e) => return Err(e.into()),
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                current_offset += bytes_read as u64;
                continue;
            }

            let record: WalRecord = match serde_json::from_str(trimmed) {
                Ok(r) => r,
                Err(e) => {
                    warn!(
                        offset = current_offset,
                        error = %e,
                        "Corrupt WAL entry during replay, stopping at corruption point",
                    );
                    break;
                }
            };

            current_offset += bytes_read as u64;

            if record.seq > seq {
                entries.push(WalEntry {
                    seq: record.seq,
                    event: record.event,
                });
            }
        }

        Ok(entries)
    }
}

#[cfg(test)]
#[path = "wal_tests.rs"]
mod tests;
